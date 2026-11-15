//! Offline place search: a browser-free gazetteer with diacritic-insensitive
//! matching and ranking.
//!
//! The data is `assets/data/places.tsv`, built from Natural Earth countries
//! and populated places by `scripts/convert_geojson.py` (format described
//! there). Matching works on a folded form of each name: lowercase, accents
//! removed (`Zürich` -> `zurich`, `São Paulo` -> `sao paulo`), punctuation
//! turned into single spaces. The index is built once when the file loads;
//! a query is then one pass over about 7,500 short strings, well under a
//! millisecond, so it is cheap enough to run on every keystroke.

use crate::camera::{MAX_ALT, MIN_ALT, TAN_HALF_FOV};

/// First line of `places.tsv`.
pub const MAGIC: &str = "MAPSPLACES1";
/// Longest query considered, in characters. Longer input is cut off.
pub const MAX_QUERY_CHARS: usize = 64;
/// Lowest altitude when framing a country, so tiny states stay in context.
const MIN_COUNTRY_ALT: f64 = 0.12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Country,
    Place,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub kind: Kind,
    pub name: String,
    /// Country name for a populated place; empty for a country.
    pub country: String,
    /// Degrees.
    pub lat: f64,
    pub lon: f64,
    pub population: u64,
    /// Angular size in degrees to frame; 0 for populated places.
    pub extent_deg: f64,
}

impl Place {
    /// Camera altitude (in globe radii) that frames this place: the whole
    /// country with some margin, or a city-scale view for a populated place.
    pub fn fly_altitude(&self) -> f64 {
        let (alt, floor) = if self.extent_deg > 0.0 {
            // Visible half-width on the ground is about alt * tan(fov/2).
            (
                0.75 * self.extent_deg.to_radians() / TAN_HALF_FOV,
                MIN_COUNTRY_ALT,
            )
        } else if self.population >= 1_000_000 {
            (0.1, MIN_ALT)
        } else {
            (0.07, MIN_ALT)
        };
        alt.clamp(floor, MAX_ALT)
    }
}

/// The ASCII letters a lowercase character folds to, or `None` to keep it.
fn fold_letter(c: char) -> Option<&'static str> {
    const TABLE: &[(&str, &str)] = &[
        ("àáâãäåāăąạảẩắằ", "a"),
        ("çćčĉċ", "c"),
        ("ďđð", "d"),
        ("èéêëēėęěệ", "e"),
        ("ğġĝ", "g"),
        ("ĥħ", "h"),
        ("ìíîïĩīįıị", "i"),
        ("ĵ", "j"),
        ("ķ", "k"),
        ("ĺļľł", "l"),
        ("ñńņň", "n"),
        ("òóôõöøōőơốồớ", "o"),
        ("ŕřŗ", "r"),
        ("śŝşšș", "s"),
        ("ţťțṭ", "t"),
        ("ùúûüũūůűų", "u"),
        ("ŵ", "w"),
        ("ýÿŷỹ", "y"),
        ("źżž", "z"),
        ("ß", "ss"),
        ("æ", "ae"),
        ("œ", "oe"),
        ("þ", "th"),
    ];
    if c.is_ascii() {
        return None;
    }
    TABLE
        .iter()
        .find(|(from, _)| from.contains(c))
        .map(|&(_, to)| to)
}

/// Fold text for matching: lowercase, accents removed, every run of
/// non-alphanumeric characters becomes one space, and the ends are trimmed.
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for c in text.chars().flat_map(char::to_lowercase) {
        // Combining marks (accents written as separate characters).
        if ('\u{300}'..='\u{36f}').contains(&c) {
            continue;
        }
        let folded = fold_letter(c);
        if folded.is_some() || c.is_alphanumeric() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            match folded {
                Some(s) => out.push_str(s),
                None => out.push(c),
            }
        } else {
            pending_space = true;
        }
    }
    out
}

/// How well a key matches a query; lower is better.
fn match_tier(key: &str, query: &str) -> Option<u8> {
    if key == query {
        Some(0)
    } else if key.starts_with(query) {
        Some(1)
    } else {
        // A later word starting with the query ("york" in "new york") ranks
        // above a match in the middle of a word.
        key.find(query)
            .map(|i| if key[..i].ends_with(' ') { 2 } else { 3 })
    }
}

/// Tier of every typo-tolerant match: below all exact/prefix/substring tiers.
const FUZZY_TIER: u8 = 4;
/// Longest key (bytes) the fuzzy matcher looks at; longer keys are cut off.
const MAX_FUZZY_KEY: usize = 96;

/// Most typos tolerated for a query of `chars` characters: none for 1-3,
/// one for 4-6, two for 7 or more.
fn max_typos(chars: usize) -> usize {
    match chars {
        0..=3 => 0,
        4..=6 => 1,
        _ => 2,
    }
}

/// Reusable DP rows for [`typo_distances`].
struct Scratch {
    rows: [[u8; MAX_FUZZY_KEY + 2]; 3],
}

/// Banded optimal-string-alignment distance (insert, delete, substitute,
/// adjacent swap, each cost 1) between `q` and a key, bounded by `k`.
/// Returns `(whole, prefix)`: the distance to the entire key and the least
/// distance to any prefix of it (`None` when above `k`). Only cells within
/// `k` of the diagonal are computed, and it stops as soon as a whole row
/// exceeds `k`, so a non-match usually costs a few cells.
fn typo_distances(key: &[u8], q: &[u8], k: usize, sc: &mut Scratch) -> (Option<u8>, Option<u8>) {
    let (n, m) = (key.len().min(MAX_FUZZY_KEY), q.len());
    // A prefix of the key needs at least m-k characters.
    if n + k < m || k > 2 {
        return (None, None);
    }
    const INF: u8 = 200;
    let kk = k as u8;
    // Row i lives in rows[i % 3]; row i-2 is the swap source.
    for (j, cell) in sc.rows[0].iter_mut().enumerate().take(k.min(n) + 1) {
        *cell = j as u8;
    }
    if k < n {
        sc.rows[0][k + 1] = INF;
    }
    for i in 1..=m {
        let (cur, p1, p2) = (i % 3, (i + 2) % 3, (i + 1) % 3);
        let lo = i.saturating_sub(k);
        let hi = (i + k).min(n);
        if lo > n {
            return (None, None);
        }
        let mut best = INF;
        if lo > 0 {
            sc.rows[cur][lo - 1] = INF;
        }
        for j in lo..=hi {
            let v = if j == 0 {
                i as u8
            } else {
                let cost = u8::from(q[i - 1] != key[j - 1]);
                let mut v = (sc.rows[p1][j - 1] + cost)
                    .min(sc.rows[p1][j] + 1)
                    .min(sc.rows[cur][j - 1] + 1);
                if i > 1 && j > 1 && q[i - 1] == key[j - 2] && q[i - 2] == key[j - 1] {
                    v = v.min(sc.rows[p2][j - 2] + 1);
                }
                v
            };
            sc.rows[cur][j] = v.min(INF);
            best = best.min(v);
        }
        if hi < n {
            sc.rows[cur][hi + 1] = INF;
        }
        if best > kk {
            return (None, None);
        }
    }
    let last = &sc.rows[m % 3];
    let lo = m.saturating_sub(k);
    let hi = (m + k).min(n);
    let prefix = (lo..=hi).map(|j| last[j]).min().filter(|&d| d <= kk);
    let whole = (n == key.len() && n <= m + k)
        .then(|| last[n])
        .filter(|&d| d <= kk);
    (whole, prefix)
}

/// Typo-tolerant score of a key against the query (lower is better): twice
/// the edit distance, plus 1 if only a prefix of the key matched, plus 1 more
/// if that prefix starts at a later word. `None` when out of range.
fn fuzzy_score(key: &str, q: &[u8], k: usize, sc: &mut Scratch) -> Option<u8> {
    let kb = key.as_bytes();
    let mut best: Option<u8> = None;
    let mut consider = |s: Option<u8>| {
        if let Some(s) = s {
            best = Some(best.map_or(s, |b| b.min(s)));
        }
    };
    let (whole, prefix) = typo_distances(kb, q, k, sc);
    consider(whole.map(|d| 2 * d));
    consider(prefix.map(|d| 2 * d + 1));
    for (i, _) in kb.iter().enumerate().filter(|&(_, &b)| b == b' ') {
        if kb.len() - i - 1 + k >= q.len() {
            consider(typo_distances(&kb[i + 1..], q, k, sc).1.map(|d| 2 * d + 2));
        }
    }
    best
}

pub struct Gazetteer {
    places: Vec<Place>,
    /// Folded search keys per place: the name, then its aliases.
    keys: Vec<Vec<String>>,
}

impl Gazetteer {
    /// Parse `places.tsv`. Malformed lines are skipped; a wrong header is an
    /// error.
    pub fn parse(text: &str) -> Result<Gazetteer, String> {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(MAGIC) {
            return Err("not a places file (bad header)".into());
        }
        let mut places = Vec::new();
        let mut keys = Vec::new();
        for line in lines {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 8 || f[1].is_empty() {
                continue;
            }
            let kind = match f[0] {
                "C" => Kind::Country,
                "P" => Kind::Place,
                _ => continue,
            };
            let (Ok(lat), Ok(lon), Ok(pop), Ok(ext)) = (
                f[3].parse::<f64>(),
                f[4].parse::<f64>(),
                f[5].parse::<u64>(),
                f[6].parse::<f64>(),
            ) else {
                continue;
            };
            if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0) {
                continue;
            }
            let mut k = vec![fold(f[1])];
            k.extend(f[7].split(';').map(fold).filter(|a| !a.is_empty()));
            if k[0].is_empty() {
                continue;
            }
            keys.push(k);
            places.push(Place {
                kind,
                name: f[1].to_string(),
                country: f[2].to_string(),
                lat,
                lon,
                population: pop,
                extent_deg: if ext.is_finite() { ext.max(0.0) } else { 0.0 },
            });
        }
        Ok(Gazetteer { places, keys })
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.places.len()
    }

    /// Up to `limit` best matches. Ranked exact name, then name prefix, then
    /// word prefix, then substring, then typo-tolerant matches (see `typo_distances`) by
    /// edit distance; ties go to the larger population. Aliases
    /// ("Russian Federation") count like names. Empty or blank queries match
    /// nothing.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Place> {
        let truncated: String = query.chars().take(MAX_QUERY_CHARS).collect();
        let q = fold(&truncated);
        if q.is_empty() || limit == 0 {
            return Vec::new();
        }
        // (tier, typo score, place index). The score is 0 below the fuzzy tier.
        let mut hits: Vec<(u8, u8, usize)> = self
            .keys
            .iter()
            .enumerate()
            .filter_map(|(i, keys)| {
                keys.iter()
                    .filter_map(|k| match_tier(k, &q))
                    .min()
                    .map(|t| (t, 0, i))
            })
            .collect();
        // Typo-tolerant matches only matter when the exact tiers leave room.
        let k = max_typos(q.chars().count());
        if k > 0 && hits.len() < limit {
            let mut matched = vec![false; self.places.len()];
            for h in &hits {
                matched[h.2] = true;
            }
            let qb = q.as_bytes();
            let mut sc = Scratch {
                rows: [[0; MAX_FUZZY_KEY + 2]; 3],
            };
            for (i, keys) in self.keys.iter().enumerate() {
                if matched[i] {
                    continue;
                }
                let s = keys
                    .iter()
                    .filter_map(|key| fuzzy_score(key, qb, k, &mut sc))
                    .min();
                if let Some(s) = s {
                    hits.push((FUZZY_TIER, s, i));
                }
            }
        }
        hits.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.cmp(&b.1))
                .then(
                    self.places[b.2]
                        .population
                        .cmp(&self.places[a.2].population),
                )
                .then(a.2.cmp(&b.2))
        });
        hits.into_iter()
            .take(limit)
            .map(|(_, _, i)| &self.places[i])
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = include_str!("../assets/data/places.tsv");

    fn names(g: &Gazetteer, q: &str, n: usize) -> Vec<String> {
        g.search(q, n).iter().map(|p| p.name.clone()).collect()
    }

    fn small() -> Gazetteer {
        Gazetteer::parse(
            "MAPSPLACES1\n\
             P\tParis\tFrance\t48.86\t2.35\t2000000\t0.0\t\n\
             P\tParis\tUnited States of America\t33.66\t-95.55\t25000\t0.0\t\n\
             P\tNew York\tUnited States of America\t40.7\t-74.0\t8000000\t0.0\tNYC\n\
             P\tYork\tUnited Kingdom\t53.96\t-1.08\t150000\t0.0\t\n\
             P\tZ\u{fc}rich\tSwitzerland\t47.37\t8.54\t400000\t0.0\t\n\
             P\tS\u{e3}o Paulo\tBrazil\t-23.55\t-46.63\t12000000\t0.0\t\n\
             P\tParisville\tNowhere\t1.0\t1.0\t900000000\t0.0\t\n\
             C\tRussia\t\t59.5\t103.7\t144000000\t77.6\tRussian Federation\n",
        )
        .unwrap()
    }

    #[test]
    fn folds_diacritics_case_and_punctuation() {
        assert_eq!(fold("Zürich"), "zurich");
        assert_eq!(fold("São Paulo"), "sao paulo");
        assert_eq!(fold("  St.  Louis-Park "), "st louis park");
        assert_eq!(fold("Łódź"), "lodz");
        assert_eq!(fold("Straße"), "strasse");
        assert_eq!(fold("Tây Ninh"), "tay ninh");
        // Combining accents (NFD) fold the same way.
        assert_eq!(fold("Zu\u{308}rich"), "zurich");
        assert_eq!(fold(""), "");
        assert_eq!(fold(" -.' "), "");
    }

    #[test]
    fn matches_without_diacritics() {
        let g = small();
        assert_eq!(names(&g, "zurich", 5), ["Zürich"]);
        assert_eq!(names(&g, "ZÜRICH", 5), ["Zürich"]);
        assert_eq!(names(&g, "sao paulo", 5), ["São Paulo"]);
        assert_eq!(names(&g, "são", 5), ["São Paulo"]);
    }

    #[test]
    fn ranks_exact_then_prefix_then_word_then_substring() {
        let g = small();
        // Exact (York) beats a later word start (New York).
        assert_eq!(names(&g, "york", 5), ["York", "New York"]);
        // Exact beats prefix regardless of population; population orders
        // equal matches.
        assert_eq!(names(&g, "paris", 5), ["Paris", "Paris", "Parisville"]);
        let r = g.search("paris", 5);
        assert_eq!(r[0].country, "France");
        assert_eq!(r[1].country, "United States of America");
        // A prefix match beats a substring match even when less populous.
        assert_eq!(names(&g, "par", 5), ["Parisville", "Paris", "Paris"]);
        let r = names(&g, "ris", 6);
        assert_eq!(r, ["Parisville", "Paris", "Paris"]);
        assert_eq!(names(&g, "ul", 5), ["São Paulo"]);
        // Prefix before substring: "o" prefixes nothing here but "sao"
        // matches São Paulo by prefix while "ao" is a substring.
        let g2 = Gazetteer::parse(
            "MAPSPLACES1\nP\tBigtown\tX\t0\t0\t900\t0\t\nP\tTown\tX\t0\t0\t10\t0\t\nP\tTownsville\tX\t0\t0\t5\t0\t\n",
        )
        .unwrap();
        assert_eq!(names(&g2, "town", 5), ["Town", "Townsville", "Bigtown"]);
    }

    #[test]
    fn matches_aliases_and_punctuated_queries() {
        let g = small();
        assert_eq!(names(&g, "russian fed", 5), ["Russia"]);
        assert_eq!(names(&g, "new-york", 5), ["New York"]);
        assert_eq!(names(&g, "nyc", 5), ["New York"]);
    }

    #[test]
    fn limits_and_degenerate_input() {
        let g = small();
        assert_eq!(g.search("paris", 2).len(), 2);
        assert!(g.search("paris", 0).is_empty());
        assert!(g.search("", 5).is_empty());
        assert!(g.search("   \t ", 5).is_empty());
        assert!(g.search("...", 5).is_empty());
        assert!(g.search("qqqq", 5).is_empty());
        // Very long input is truncated, not slow or panicking.
        assert!(g.search(&"paris ".repeat(10_000), 5).is_empty());
        assert!(g.search(&"a".repeat(1_000_000), 5).is_empty());
        // Truncation cuts at a character boundary.
        assert!(g.search(&"ü".repeat(200), 5).is_empty());
    }

    #[test]
    fn parse_rejects_bad_header_and_skips_bad_lines() {
        assert!(Gazetteer::parse("hello\n").is_err());
        assert!(Gazetteer::parse("").is_err());
        let g = Gazetteer::parse(
            "MAPSPLACES1\nP\tA\tB\tnan\t0\t1\t0\t\nP\tOnly\tfew\nP\tOk\tX\t10\t20\t5\t0\t\nP\tFar\tX\t95\t0\t5\t0\t\nP\tInf\tX\t0\tinf\t5\t0\t\n",
        )
        .unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g.search("ok", 5)[0].name, "Ok");
    }

    #[test]
    fn fly_altitude_frames_the_place() {
        let g = small();
        let russia = g.search("russia", 1)[0];
        let zurich = g.search("zurich", 1)[0];
        assert!(russia.fly_altitude() > zurich.fly_altitude());
        assert!(russia.fly_altitude() <= MAX_ALT);
        assert!(zurich.fly_altitude() >= MIN_ALT);
        let tiny = Place {
            kind: Kind::Country,
            name: "Tiny".into(),
            country: String::new(),
            lat: 0.0,
            lon: 0.0,
            population: 1,
            extent_deg: 0.1,
        };
        assert_eq!(tiny.fly_altitude(), MIN_COUNTRY_ALT);
        let huge = Place {
            extent_deg: 1e9,
            ..tiny
        };
        assert_eq!(huge.fly_altitude(), MAX_ALT);
    }

    // --- bundled data ---

    #[test]
    fn bundled_data_parses_in_full() {
        let g = Gazetteer::parse(DATA).unwrap();
        assert!(g.len() > 7000, "only {} places", g.len());
        assert_eq!(g.len(), DATA.lines().count() - 1, "every line should parse");
        assert!(g.places.iter().any(|p| p.kind == Kind::Country));
    }

    #[test]
    fn bundled_names_fold_to_plain_ascii() {
        // A letter the fold table misses would be unsearchable on a plain
        // keyboard. Names in non-Latin scripts would trip this too.
        let g = Gazetteer::parse(DATA).unwrap();
        for k in g.keys.iter().flatten() {
            assert!(
                k.chars().all(|c| c.is_ascii_alphanumeric() || c == ' '),
                "unfolded character in {k:?}"
            );
        }
    }

    #[test]
    fn acceptance_queries() {
        let g = Gazetteer::parse(DATA).unwrap();
        let top = names(&g, "pari", 8);
        assert!(top.iter().any(|n| n == "Paris"), "{top:?}");
        let united = names(&g, "united", 8);
        for want in [
            "United States of America",
            "United Kingdom",
            "United Arab Emirates",
        ] {
            assert!(united.iter().any(|n| n == want), "{want} not in {united:?}");
        }
        assert_eq!(names(&g, "zurich", 1), ["Zürich"]);
        assert_eq!(names(&g, "sao paulo", 1), ["São Paulo"]);
        assert_eq!(g.search("london", 1)[0].country, "United Kingdom");
        // Frames use the mainland box, not one spanning the antimeridian.
        let us = g.search("united states", 1)[0];
        assert!(us.extent_deg > 20.0 && us.extent_deg < 90.0);
        let ru = g.search("russia", 1)[0];
        assert!((ru.lon - 100.0).abs() < 30.0);
    }

    #[test]
    fn typo_distance_is_osa_within_band() {
        let mut sc = Scratch {
            rows: [[0; MAX_FUZZY_KEY + 2]; 3],
        };
        let mut d =
            |key: &str, q: &str, k| typo_distances(key.as_bytes(), q.as_bytes(), k, &mut sc);
        assert_eq!(d("paris", "pairs", 1), (Some(1), Some(1)));
        assert_eq!(d("paris", "paris", 1).0, Some(0));
        assert_eq!(d("zurich", "zurihc", 1).1, Some(1));
        assert_eq!(d("zurich", "zuric", 1), (Some(1), Some(0)));
        assert_eq!(d("paris", "pxxis", 1), (None, None));
        assert_eq!(d("paris", "pxxis", 2).0, Some(2));
        assert_eq!(d("ab", "abcdefg", 2), (None, None));
        assert_eq!(d("a", "abc", 2).0, Some(2));
    }

    #[test]
    fn fuzzy_finds_typos() {
        let g = small();
        assert_eq!(names(&g, "zuric", 5), ["Zürich"]);
        assert_eq!(names(&g, "zurihc", 5), ["Zürich"]);
        assert_eq!(names(&g, "züriich", 5), ["Zürich"]);
        assert_eq!(names(&g, "sao paolo", 5), ["São Paulo"]);
        assert_eq!(names(&g, "pairs", 1)[0], "Paris");
        assert_eq!(g.search("pairs", 1)[0].country, "France");
        assert_eq!(names(&g, "russan", 5), ["Russia"]);
        assert_eq!(names(&g, "russian federaton", 5), ["Russia"]);
        // A typo in a later word of a longer name.
        assert_eq!(names(&g, "yrok", 5), ["York", "New York"]);
        assert_eq!(names(&g, "new yrok", 5), ["New York"]);
    }

    #[test]
    fn fuzzy_ranks_below_exact_tiers_then_by_distance() {
        let g = Gazetteer::parse(
            "MAPSPLACES1\n\
             P\tBerlin\tX\t0\t0\t10\t0\t\n\
             P\tBerliner Tiny\tX\t0\t0\t1\t0\t\n\
             P\tBerline\tX\t0\t0\t5\t0\t\n\
             P\tBarlin\tX\t0\t0\t99999\t0\t\n\
             P\tBxxlin\tX\t0\t0\t99999999\t0\t\n",
        )
        .unwrap();
        // "berlin": exact, then prefix match, then fuzzy by distance.
        assert_eq!(
            names(&g, "berlin", 9),
            ["Berlin", "Berline", "Berliner Tiny", "Barlin"]
        );
        // Equal distance falls back to population.
        let g = Gazetteer::parse(
            "MAPSPLACES1\nP\tBarlin\tX\t0\t0\t5\t0\t\nP\tBerlon\tX\t0\t0\t50\t0\t\n",
        )
        .unwrap();
        assert_eq!(names(&g, "berlin", 9), ["Berlon", "Barlin"]);
    }

    #[test]
    fn short_queries_do_not_go_fuzzy() {
        let g = Gazetteer::parse(DATA).unwrap();
        // 1-3 characters: exact tiers only, never "everything".
        for q in ["ab", "xq", "zzz", "qqq"] {
            let all = g.search(q, 10_000);
            for p in all {
                let ok = g.keys[g.places.iter().position(|x| std::ptr::eq(x, p)).unwrap()]
                    .iter()
                    .any(|k| match_tier(k, q).is_some());
                assert!(ok, "{q} fuzzily matched {}", p.name);
            }
        }
        // A 4-letter query tolerates one typo, not wholesale noise.
        assert!(g.search("qxqx", 10_000).len() < 50);
        assert!(g.search("abcdefghijkl", 10_000).is_empty());
    }

    #[test]
    fn fuzzy_on_bundled_data() {
        let g = Gazetteer::parse(DATA).unwrap();
        assert_eq!(names(&g, "zuric", 1), ["Zürich"]);
        assert_eq!(names(&g, "zurihc", 1), ["Zürich"]);
        assert_eq!(names(&g, "sao paolo", 1), ["São Paulo"]);
        assert_eq!(g.search("pairs", 1)[0].name, "Paris");
        assert_eq!(g.search("pairs", 1)[0].country, "France");
        assert_eq!(g.search("lodnon", 1)[0].country, "United Kingdom");
        assert_eq!(names(&g, "Ünited Stats", 1), ["United States of America"]);
    }

    #[test]
    fn fuzzy_handles_odd_input() {
        let g = Gazetteer::parse(DATA).unwrap();
        for q in [
            "",
            " ",
            "\u{0}\u{0}\u{0}\u{0}",
            "NaN",
            "inf",
            "-inf",
            "¿¿¿¿¿¿¿¿",
            "日本語東京都市",
            "ü",
            "üüüüüüüüü",
            &"x".repeat(5000),
            &"é".repeat(300),
        ] {
            let r = g.search(q, 8);
            assert!(r.len() <= 8);
        }
        assert!(g.search("", 8).is_empty());
        // A long key is cut off, not indexed out of range.
        let long = format!("MAPSPLACES1\nP\t{}\tX\t0\t0\t1\t0\t\n", "a".repeat(500));
        let g = Gazetteer::parse(&long).unwrap();
        assert!(g.search(&"b".repeat(8), 3).is_empty());
        assert_eq!(g.search(&"a".repeat(7), 3).len(), 1);
    }

    /// Worst-case typing cost: run with `cargo test --release -- --ignored
    /// --nocapture fuzzy_benchmark`.
    #[test]
    #[ignore]
    fn fuzzy_benchmark() {
        let g = Gazetteer::parse(DATA).unwrap();
        let queries = [
            "zurihc",
            "pairs",
            "sao paolo",
            "new yrok",
            "qwertyuiop",
            "abcdefghijklmnop",
            "mmmmmmmm",
            "san francisko",
            "united stats of amrica",
            "saint petersburgh",
            "xxxx",
            "bejing",
            "kathmandoo",
        ];
        let mut worst = std::time::Duration::ZERO;
        for q in queries {
            let start = std::time::Instant::now();
            for _ in 0..50 {
                std::hint::black_box(g.search(q, 6));
            }
            let per = start.elapsed() / 50;
            worst = worst.max(per);
            eprintln!("{q:>26}: {per:?}");
        }
        eprintln!("worst: {worst:?}");
        assert!(worst.as_millis() < 5);
    }

    #[test]
    fn typing_is_fast() {
        let g = Gazetteer::parse(DATA).unwrap();
        let queries = [
            "p", "pa", "par", "pari", "paris", "new yo", "san", "zzz", "é", "pairs", "zurihc",
        ];
        let rounds = 20;
        let start = std::time::Instant::now();
        for _ in 0..rounds {
            for q in queries {
                std::hint::black_box(g.search(q, 8));
            }
        }
        let per = start.elapsed() / (rounds * queries.len() as u32);
        eprintln!("search: {per:?} per query over {} places", g.len());
        // Generous: debug builds on slow CI. Release is far below 1 ms.
        assert!(per.as_millis() < 25, "{per:?} per query");
    }
}
