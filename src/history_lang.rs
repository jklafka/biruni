//! The historical borders colored by language family: Seshat's languages
//! and linguistic families of past polities, joined to Cliopatria's records.
//! Browser-free. The data is `assets/data/history_languages.tsv`, built by
//! `scripts/convert_seshat_languages.py` (format described there).
//!
//! The fills use the theme color table (`u_theme`, see
//! `Renderer::set_fill_palette`): the largest families get their own
//! colors, in the file's order (most polity records first), so a family
//! keeps its color through the years; the rest share [`OTHER`], and
//! polities Seshat has no value for are [`NO_DATA`].

use crate::history::{Record, Timeline};
use std::collections::HashMap;

pub const MAGIC: &str = "MAPSHISTLANG1";
/// Colors of the first families (sRGB), then the other families and no
/// data: [`SLOTS`] entries in all, the size of `u_theme`.
pub const PALETTE: [[u8; 3]; SLOTS] = [
    [214, 96, 77],   // Indo-European: brick
    [227, 184, 76],  // Afro-Asiatic: ochre
    [92, 160, 116],  // Sino-Tibetan: green
    [94, 138, 199],  // Turkic: blue
    [176, 112, 176], // Dravidian: purple
    [77, 172, 178],  // Mongolic: teal
    [222, 140, 172], // Japonic: pink
    [150, 112, 72],  // Niger-Congo: brown
    [170, 196, 92],  // Austroasiatic: lime
    [240, 150, 90],  // the tenth family: orange
    [150, 150, 140], // other families
    [62, 66, 74],    // no data
];
pub const SLOTS: usize = 12;
/// The palette slot of families past the first `SLOTS - 2`.
pub const OTHER: u8 = (SLOTS - 2) as u8;
/// The palette slot of polities without data.
pub const NO_DATA: u8 = (SLOTS - 1) as u8;

/// A polity's years and what Seshat says about its languages.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub from: i64,
    pub to: i64,
    /// Indexes into `HistoryLanguages::families`, the main family first.
    pub families: Vec<u16>,
    /// "Latin; Greek".
    pub languages: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryLanguages {
    pub families: Vec<String>,
    /// By Cliopatria polity name.
    pub entries: HashMap<String, Vec<Entry>>,
}

/// Parse `history_languages.tsv`. Malformed lines are skipped; a wrong
/// header is an error.
pub fn parse(text: &str) -> Result<HistoryLanguages, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(MAGIC) {
        return Err("not a history languages file (bad header)".into());
    }
    let mut out = HistoryLanguages::default();
    for line in lines {
        let f: Vec<&str> = line.split('\t').collect();
        match f.as_slice() {
            ["F", name] if out.entries.is_empty() => out.families.push(name.to_string()),
            ["P", name, from, to, fams, langs] if !name.is_empty() => {
                let (Ok(from), Ok(to)) = (from.parse::<i64>(), to.parse::<i64>()) else {
                    continue;
                };
                let families: Option<Vec<u16>> = fams
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        s.parse::<u16>()
                            .ok()
                            .filter(|&i| (i as usize) < out.families.len())
                    })
                    .collect();
                let Some(families) = families else { continue };
                if from > to || (families.is_empty() && langs.is_empty()) {
                    continue;
                }
                out.entries
                    .entry(name.to_string())
                    .or_default()
                    .push(Entry {
                        from,
                        to,
                        families,
                        languages: langs.to_string(),
                    });
            }
            _ => {}
        }
    }
    Ok(out)
}

impl HistoryLanguages {
    /// The entry of a polity record: its name's entry whose years hold the
    /// record's.
    pub fn entry(&self, name: &str, from: i64, to: i64) -> Option<&Entry> {
        self.entries
            .get(name)?
            .iter()
            .find(|e| e.from <= from && to <= e.to)
    }

    pub fn record_entry(&self, t: &Timeline, r: &Record) -> Option<&Entry> {
        self.entry(t.name(r), r.from, r.to)
    }

    /// The palette slot of a family index.
    pub fn slot(family: u16) -> u8 {
        if (family as usize) < OTHER as usize {
            family as u8
        } else {
            OTHER
        }
    }

    /// The palette slot of a record: its main family's.
    pub fn record_slot(&self, t: &Timeline, r: &Record) -> u8 {
        self.record_entry(t, r)
            .and_then(|e| e.families.first())
            .map_or(NO_DATA, |&f| Self::slot(f))
    }

    /// Palette slot of each raster ID (index 0 unused), as
    /// `Timeline::id_colors` gives the political colors.
    pub fn id_slots(&self, t: &Timeline, active: &[usize]) -> [u8; 256] {
        let mut out = [NO_DATA; 256];
        for (pos, &ri) in active.iter().enumerate().take(255) {
            out[pos + 1] = self.record_slot(t, &t.records[ri]);
        }
        out
    }

    /// HUD text after a polity's name: "Latin; Greek (Indo-European)", or
    /// that Seshat has no data.
    pub fn describe(&self, t: &Timeline, r: &Record) -> String {
        let Some(e) = self.record_entry(t, r) else {
            return "no language data".into();
        };
        let fams: Vec<&str> = e
            .families
            .iter()
            .filter_map(|&f| self.families.get(f as usize).map(String::as_str))
            .collect();
        match (e.languages.is_empty(), fams.is_empty()) {
            (false, false) => format!("{} ({})", e.languages, fams.join(", ")),
            (false, true) => e.languages.clone(),
            _ => fams.join(", "),
        }
    }

    /// The legend: (sRGB, label) for the families with their own color,
    /// the other families and no data.
    pub fn legend(&self) -> Vec<([u8; 3], String)> {
        let mut rows: Vec<([u8; 3], String)> = self
            .families
            .iter()
            .take(OTHER as usize)
            .enumerate()
            .map(|(i, f)| (PALETTE[i], f.clone()))
            .collect();
        if self.families.len() > OTHER as usize {
            rows.push((PALETTE[OTHER as usize], "Other families".into()));
        }
        rows.push((PALETTE[NO_DATA as usize], "No data in Seshat".into()));
        rows
    }
}

/// `&langfam=1` in a URL hash: the historical borders colored by family.
pub fn parse_hash(hash: &str) -> bool {
    hash.trim_start_matches('#')
        .split('&')
        .skip(1)
        .any(|kv| kv == "langfam=1")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TSV: &str = "MAPSHISTLANG1
F\tIndo-European
F\tAfro-Asiatic
P\tRoman Empire\t-30\t286\t0\tLatin
P\tRoman Empire\t287\t394\t0\tLatin; Greek
P\tPtolemaic Kingdom\t-291\t-198\t1,0\tGreek; demotic Egyptian
P\tLanguage only\t1\t2\t\tPunic
P\tBad family\t1\t2\t5\tX
P\tBackwards\t5\t1\t0\tX
P\tEmpty\t1\t2\t\t
P\tShort\t1
";

    #[test]
    fn parses_and_finds_entries() {
        let h = parse(TSV).unwrap();
        assert_eq!(h.families, ["Indo-European", "Afro-Asiatic"]);
        assert_eq!(h.entries.len(), 3, "bad lines skipped");
        assert_eq!(
            h.entry("Roman Empire", 300, 350).unwrap().languages,
            "Latin; Greek"
        );
        assert_eq!(h.entry("Roman Empire", -30, 100).unwrap().from, -30);
        assert!(
            h.entry("Roman Empire", 280, 300).is_none(),
            "spans two entries"
        );
        assert!(h.entry("Roman Empire", 400, 410).is_none());
        assert!(h.entry("Carthage", 0, 1).is_none());
        assert!(parse("MAPSLANGS2\n").is_err());
    }

    #[test]
    fn slots_and_legend() {
        let h = parse(TSV).unwrap();
        assert_eq!(HistoryLanguages::slot(0), 0);
        assert_eq!(HistoryLanguages::slot(9), 9);
        assert_eq!(HistoryLanguages::slot(10), OTHER);
        assert_eq!(HistoryLanguages::slot(300), OTHER);
        let legend = h.legend();
        assert_eq!(legend.len(), 3);
        assert_eq!(legend[1], (PALETTE[1], "Afro-Asiatic".into()));
        assert_eq!(legend[2].1, "No data in Seshat");
        let mut many = h.clone();
        many.families = (0..14).map(|i| format!("F{i}")).collect();
        let legend = many.legend();
        assert_eq!(legend.len(), SLOTS);
        assert_eq!(legend[10].1, "Other families");
    }

    #[test]
    fn the_committed_file_parses_and_joins_cliopatria() {
        let h = parse(include_str!("../assets/data/history_languages.tsv")).unwrap();
        assert_eq!(h.families[0], "Indo-European");
        let t =
            crate::history::decode(include_bytes!("../assets/data/history_borders.bin")).unwrap();
        // Rome in 117 is Latin-speaking and Indo-European.
        let active = t.active(117);
        let rome = active
            .iter()
            .map(|&i| &t.records[i])
            .find(|r| t.name(r) == "Roman Empire")
            .expect("Rome in 117");
        assert_eq!(h.record_slot(&t, rome), 0);
        assert!(
            h.describe(&t, rome).starts_with("Latin"),
            "{}",
            h.describe(&t, rome)
        );
        // Some polities in 1500 have data, some do not.
        let slots = h.id_slots(&t, &t.active(1500));
        let n = t.active(1500).len();
        let with = slots[1..=n].iter().filter(|&&s| s != NO_DATA).count();
        assert!(with >= 10 && with < n, "{with} of {n}");
        // Every entry names a polity of the borders file.
        let names: std::collections::HashSet<&str> = t.names.iter().map(String::as_str).collect();
        assert!(h.entries.keys().all(|k| names.contains(k.as_str())));
    }

    #[test]
    fn describe_and_hash() {
        let h = parse(TSV).unwrap();
        let e = h.entry("Ptolemaic Kingdom", -250, -200).unwrap();
        assert_eq!(e.families, [1, 0]);
        assert!(parse_hash("#1,2,3&year=117&langfam=1"));
        assert!(!parse_hash("#1,2,3&year=117"));
        assert!(!parse_hash("#langfam=1"));
    }
}
