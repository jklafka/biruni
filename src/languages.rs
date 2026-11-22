//! The languages layer: Glottolog's languages as dots colored by family,
//! with names when zoomed in. Browser-free (the overlay draws what this
//! returns). The data is `assets/data/languages.tsv`, built by
//! `scripts/convert_glottolog.py` (format described there).

use crate::camera::Camera;
use crate::labels::facing;
use crate::stars::{overlaps, Rect};

/// First line of `languages.tsv`.
pub const MAGIC: &str = "MAPSLANGS2";
/// The largest families get their own color, in order of size; the rest
/// share [`OTHER_COLOR`], and isolates have [`ISOLATE_COLOR`].
pub const FAMILY_COLORS: [&str; 12] = [
    "#e6194b", "#3cb44b", "#ffe119", "#4363d8", "#f58231", "#42d4f4", "#f032e6", "#bfef45",
    "#fabed4", "#469990", "#dcbeff", "#9a6324",
];
pub const OTHER_COLOR: &str = "#a9a9a9";
pub const ISOLATE_COLOR: &str = "#ffffff";
/// Endangerment, not endangered to extinct: green through red, then violet.
pub const ENDANGERMENT_COLORS: [&str; 6] = [
    "#3cb44b", "#bfef45", "#ffe119", "#f58231", "#e6194b", "#b07cff",
];
/// The id of the endangerment variable.
pub const ENDANGERMENT: &str = "aes";
/// A yes/no feature: no, yes.
pub const NO_COLOR: &str = "#4363d8";
pub const YES_COLOR: &str = "#f58231";
/// Dot radius and the hit radius around it, CSS pixels.
pub const DOT_PX: f64 = 2.6;
pub const HIT_PX: f64 = 7.0;
/// Names show below this altitude (globe radii, about 2,500 km), at most
/// [`MAX_NAMES`] at once.
pub const NAMES_BELOW_ALT: f64 = 0.4;
pub const MAX_NAMES: usize = 80;

/// A variable the dots can be colored by: endangerment or a Grambank
/// feature.
#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    pub id: String,
    pub title: String,
    /// The source's question or description, for the legend.
    pub question: String,
    /// (code, label), by code.
    pub codes: Vec<(u8, String)>,
}

impl Variable {
    /// Grambank's yes/no features.
    pub fn is_binary(&self) -> bool {
        self.codes.len() == 2
            && self.codes[0] == (0, "No".into())
            && self.codes[1] == (1, "Yes".into())
    }

    /// The color of a code (`None` for an unknown one).
    pub fn color(&self, code: u8) -> Option<&'static str> {
        let i = self.codes.iter().position(|(c, _)| *c == code)?;
        if self.id == ENDANGERMENT {
            ENDANGERMENT_COLORS.get(i).copied()
        } else if self.is_binary() {
            Some([NO_COLOR, YES_COLOR][i])
        } else {
            FAMILY_COLORS.get(i).copied()
        }
    }

    pub fn label(&self, code: u8) -> Option<&str> {
        self.codes
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, l)| l.as_str())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Language {
    pub code: String,
    pub name: String,
    /// Index into `Languages::families`; `None` for an isolate.
    pub family: Option<u16>,
    /// Radians.
    pub lat: f64,
    pub lon: f64,
    pub iso: String,
    pub area: String,
    /// One code per variable; `None` when the source has no (known) value.
    pub values: Vec<Option<u8>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Languages {
    pub list: Vec<Language>,
    /// Family names, largest first (by number of languages), ties by name.
    pub families: Vec<String>,
    pub family_sizes: Vec<usize>,
    pub isolates: usize,
    pub variables: Vec<Variable>,
}

/// Parse `languages.tsv`. Malformed lines are skipped; a wrong header is an
/// error.
pub fn parse(text: &str) -> Result<Languages, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(MAGIC) {
        return Err("not a languages file (bad header)".into());
    }
    let mut variables = Vec::new();
    let mut raw = Vec::new();
    let mut counts: std::collections::HashMap<&str, usize> = Default::default();
    for line in lines {
        let f: Vec<&str> = line.split('\t').collect();
        match f.first() {
            Some(&"V") if f.len() == 5 && raw.is_empty() => {
                let codes = f[4]
                    .split('|')
                    .filter_map(|p| {
                        let (n, label) = p.split_once('=')?;
                        Some((n.parse::<u8>().ok()?, label.to_string()))
                    })
                    .collect();
                variables.push(Variable {
                    id: f[1].to_string(),
                    title: f[2].to_string(),
                    question: f[3].to_string(),
                    codes,
                });
            }
            Some(&"L") if f.len() == 8 + variables.len() && !f[2].is_empty() => {
                let (Ok(lat), Ok(lon)) = (f[4].parse::<f64>(), f[5].parse::<f64>()) else {
                    continue;
                };
                if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0)
                {
                    continue;
                }
                if !f[3].is_empty() {
                    *counts.entry(f[3]).or_default() += 1;
                }
                raw.push((f, lat, lon));
            }
            _ => {}
        }
    }
    let mut fams: Vec<(&str, usize)> = counts.into_iter().collect();
    fams.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let index: std::collections::HashMap<&str, u16> = fams
        .iter()
        .enumerate()
        .map(|(i, (n, _))| (*n, i as u16))
        .collect();
    let list: Vec<Language> = raw
        .into_iter()
        .map(|(f, lat, lon)| Language {
            code: f[1].to_string(),
            name: f[2].to_string(),
            family: index.get(f[3]).copied(),
            lat: lat.to_radians(),
            lon: lon.to_radians(),
            iso: f[6].to_string(),
            area: f[7].to_string(),
            values: f[8..]
                .iter()
                .zip(&variables)
                .map(|(v, var)| {
                    let code = v.parse::<u8>().ok()?;
                    var.label(code).is_some().then_some(code)
                })
                .collect(),
        })
        .collect();
    let isolates = list.iter().filter(|l| l.family.is_none()).count();
    Ok(Languages {
        list,
        families: fams.iter().map(|(n, _)| n.to_string()).collect(),
        family_sizes: fams.iter().map(|(_, c)| *c).collect(),
        isolates,
        variables,
    })
}

/// A language on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    pub index: usize,
    pub x: f64,
    pub y: f64,
}

/// The points (lat, lon in radians, by index) on screen and on the near
/// side of the globe. Shared with the societies layer.
pub fn screen_dots(cam: &Camera, points: impl Iterator<Item = (f64, f64)>) -> Vec<Dot> {
    points
        .enumerate()
        .filter(|(_, (lat, lon))| facing(cam, *lat, *lon) > 0.0)
        .filter_map(|(index, (lat, lon))| {
            let (x, y) = cam.project(lat, lon)?;
            let inside = (-DOT_PX..cam.width + DOT_PX).contains(&x)
                && (-DOT_PX..cam.height + DOT_PX).contains(&y);
            inside.then_some(Dot { index, x, y })
        })
        .collect()
}

/// Names right of their dots: clear of `avoid` and of each other, in dot
/// order, at most [`MAX_NAMES`]. Shared with the societies layer.
pub fn place_names(
    cam: &Camera,
    dots: &[Dot],
    name: impl Fn(usize) -> String,
    avoid: &[Rect],
) -> Vec<(usize, f64, f64)> {
    let mut placed = avoid.to_vec();
    let mut out = Vec::new();
    for d in dots {
        let text = name(d.index);
        let (x, y) = (d.x + DOT_PX + 4.0, d.y);
        let r = (x, y - 7.0, x + 6.2 * text.chars().count() as f64, y + 7.0);
        if r.2 > cam.width || r.1 < 0.0 || r.3 > cam.height {
            continue;
        }
        if placed.iter().any(|p| overlaps(&r, p)) {
            continue;
        }
        placed.push(r);
        out.push((d.index, x, y));
        if out.len() == MAX_NAMES {
            break;
        }
    }
    out
}

/// The dot nearest a screen point, within [`HIT_PX`].
pub fn nearest(dots: &[Dot], x: f64, y: f64) -> Option<usize> {
    dots.iter()
        .map(|d| ((d.x - x).hypot(d.y - y), d.index))
        .filter(|(dist, _)| *dist <= HIT_PX)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, i)| i)
}

impl Languages {
    /// The index of the variable with this id.
    pub fn variable(&self, id: &str) -> Option<usize> {
        self.variables.iter().position(|v| v.id == id)
    }

    /// The code of language `i` for variable `v`.
    pub fn value(&self, v: usize, i: usize) -> Option<u8> {
        self.list[i].values.get(v).copied().flatten()
    }

    /// CSS color of a language's dot: by family (`by` is `None`), or by
    /// variable `by` (`None` when the language has no value for it).
    pub fn color(&self, by: Option<usize>, i: usize) -> Option<&'static str> {
        match by {
            None => Some(match self.list[i].family {
                None => ISOLATE_COLOR,
                Some(f) => FAMILY_COLORS
                    .get(f as usize)
                    .copied()
                    .unwrap_or(OTHER_COLOR),
            }),
            Some(v) => self.variables.get(v)?.color(self.value(v, i)?),
        }
    }

    pub fn family_name(&self, i: usize) -> &str {
        match self.list[i].family {
            None => "isolate",
            Some(f) => &self.families[f as usize],
        }
    }

    /// The languages on screen and on the near side of the globe; by a
    /// variable, only those with a value for it.
    pub fn dots(&self, cam: &Camera, by: Option<usize>) -> Vec<Dot> {
        let mut dots = screen_dots(cam, self.list.iter().map(|l| (l.lat, l.lon)));
        if by.is_some() {
            dots.retain(|d| self.color(by, d.index).is_some());
        }
        dots
    }

    /// Names right of their dots when zoomed in (see [`place_names`]).
    pub fn names(&self, cam: &Camera, dots: &[Dot], avoid: &[Rect]) -> Vec<(usize, f64, f64)> {
        if cam.alt > NAMES_BELOW_ALT {
            return Vec::new();
        }
        place_names(cam, dots, |i| self.list[i].name.clone(), avoid)
    }

    /// The dot nearest a screen point, within [`HIT_PX`].
    pub fn hit(&self, dots: &[Dot], x: f64, y: f64) -> Option<usize> {
        nearest(dots, x, y)
    }

    /// HUD text: "Basque (isolate, Eurasia)", and by a variable
    /// "Basque (isolate, Eurasia) · Endangerment: Threatened".
    pub fn describe(&self, by: Option<usize>, i: usize) -> String {
        let l = &self.list[i];
        let mut parts = vec![self.family_name(i).to_string()];
        if !l.area.is_empty() {
            parts.push(l.area.replace(';', ", "));
        }
        let base = format!("{} ({})", l.name, parts.join(", "));
        let Some(var) = by.and_then(|v| self.variables.get(v)) else {
            return base;
        };
        let label = by
            .and_then(|v| self.value(v, i))
            .and_then(|c| var.label(c))
            .unwrap_or("no data");
        format!("{base} · {}: {label}", var.title)
    }

    /// The legend: (color, label) for the colored families, then the rest
    /// and the isolates; or by variable `by`, its codes with counts.
    pub fn legend(&self, by: Option<usize>) -> Vec<(&'static str, String)> {
        if let Some(var) = by.and_then(|v| self.variables.get(v)) {
            let v = by.unwrap_or(0);
            return var
                .codes
                .iter()
                .filter_map(|(c, label)| {
                    let n = (0..self.list.len())
                        .filter(|&i| self.value(v, i) == Some(*c))
                        .count();
                    Some((var.color(*c)?, format!("{label} ({n})")))
                })
                .collect();
        }
        let mut rows: Vec<(&'static str, String)> = self
            .families
            .iter()
            .zip(&self.family_sizes)
            .zip(FAMILY_COLORS)
            .map(|((n, c), color)| (color, format!("{n} ({c})")))
            .collect();
        let rest: usize = self.family_sizes.iter().skip(FAMILY_COLORS.len()).sum();
        if rest > 0 {
            let n = self.families.len().saturating_sub(FAMILY_COLORS.len());
            rows.push((OTHER_COLOR, format!("{n} other families ({rest})")));
        }
        rows.push((ISOLATE_COLOR, format!("Isolates ({})", self.isolates)));
        rows
    }

    /// The number of languages with no value for variable `v`.
    pub fn without_data(&self, v: usize) -> usize {
        (0..self.list.len())
            .filter(|&i| self.value(v, i).is_none())
            .count()
    }

    /// The languages as search entries ("Language · family").
    pub fn places(&self) -> Vec<crate::search::Place> {
        self.list
            .iter()
            .enumerate()
            .map(|(i, l)| crate::search::Place {
                kind: crate::search::Kind::Language,
                name: l.name.clone(),
                country: format!("Language · {}", self.family_name(i)),
                lat: l.lat.to_degrees(),
                lon: l.lon.to_degrees(),
                population: 0,
                extent_deg: 0.0,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TSV: &str = "MAPSLANGS2
V\taes\tEndangerment\tStatus\t1=Not endangered|2=Threatened|6=Extinct
V\tGB020\tDefinite articles\tAre there?\t0=No|1=Yes
V\tGB024\tNumeral and noun order\tOrder?\t1=Num-N|2=N-Num|3=Both
L\tbasq1248\tBasque\t\t43.2787\t-1.3164\teus\tEurasia\t2\t1\t
L\tstan1293\tEnglish\tIndo-European\t53.0\t-1.0\teng\tEurasia\t1\t1\t1
L\tstan1288\tSpanish\tIndo-European\t40.4414\t-1.1170\tspa\tEurasia\t1\t\t7
L\tswah1253\tSwahili\tAtlantic-Congo\t-8.5\t37.0\tswh\tAfrica\t\t0\t2
L\tbad\t\tX\t1\t1\t\tAfrica\t\t\t
L\tnan1\tBroken\tX\tnorth\t1\t\tAfrica\t\t\t
L\tfar1\tFar\tX\t95\t1\t\tAfrica\t\t\t
L\tshort\tShort\tX\t1\t1\t\tAfrica
";

    #[test]
    fn parses_and_ranks_families() {
        let l = parse(TSV).unwrap();
        assert_eq!(l.list.len(), 4, "bad rows skipped");
        assert_eq!(l.families, ["Indo-European", "Atlantic-Congo"]);
        assert_eq!(l.family_sizes, [2, 1]);
        assert_eq!(l.isolates, 1);
        assert_eq!(l.color(None, 0), Some(ISOLATE_COLOR));
        assert_eq!(l.color(None, 1), Some(FAMILY_COLORS[0]));
        assert_eq!(l.color(None, 3), Some(FAMILY_COLORS[1]));
        assert_eq!(l.describe(None, 0), "Basque (isolate, Eurasia)");
        assert_eq!(l.describe(None, 3), "Swahili (Atlantic-Congo, Africa)");
        assert!(parse("MAPSLANGS1\n").is_err());
        let legend = l.legend(None);
        assert_eq!(
            legend[0],
            (FAMILY_COLORS[0], "Indo-European (2)".to_string())
        );
        assert_eq!(legend.last().unwrap().1, "Isolates (1)");
    }

    #[test]
    fn variables_color_and_describe() {
        let l = parse(TSV).unwrap();
        assert_eq!(l.variables.len(), 3);
        let (aes, gb020, gb024) = (Some(0), Some(1), Some(2));
        assert_eq!(l.variable("GB024"), gb024);
        assert_eq!(l.variable("GB999"), None);
        assert!(l.variables[1].is_binary() && !l.variables[2].is_binary());
        assert_eq!(
            l.list[2].values,
            vec![Some(1), None, None],
            "an unknown code is no data"
        );
        // Endangerment ramp by position, not by code.
        assert_eq!(l.color(aes, 0), Some(ENDANGERMENT_COLORS[1]));
        assert_eq!(l.color(aes, 3), None);
        assert_eq!(l.color(gb020, 3), Some(NO_COLOR));
        assert_eq!(l.color(gb020, 1), Some(YES_COLOR));
        assert_eq!(l.color(gb024, 3), Some(FAMILY_COLORS[1]));
        assert_eq!(l.color(Some(9), 0), None, "no such variable");
        assert_eq!(
            l.describe(aes, 0),
            "Basque (isolate, Eurasia) · Endangerment: Threatened"
        );
        assert_eq!(
            l.describe(gb024, 0),
            "Basque (isolate, Eurasia) · Numeral and noun order: no data"
        );
        let legend = l.legend(aes);
        assert_eq!(
            legend,
            vec![
                (ENDANGERMENT_COLORS[0], "Not endangered (2)".to_string()),
                (ENDANGERMENT_COLORS[1], "Threatened (1)".to_string()),
                (ENDANGERMENT_COLORS[2], "Extinct (0)".to_string()),
            ]
        );
        assert_eq!(l.without_data(0), 1);
        assert_eq!(l.without_data(2), 2);
    }

    #[test]
    fn dots_names_and_hits_on_screen() {
        let l = parse(TSV).unwrap();
        let mut cam = Camera::new(47.0, 0.0, 0.39);
        cam.width = 800.0;
        cam.height = 600.0;
        let dots = l.dots(&cam, None);
        let names: Vec<&str> = dots.iter().map(|d| l.list[d.index].name.as_str()).collect();
        assert_eq!(
            names,
            ["Basque", "English", "Spanish"],
            "Swahili is off screen"
        );
        // By a variable, languages without a value are not drawn.
        let by: Vec<usize> = l.dots(&cam, Some(1)).iter().map(|d| d.index).collect();
        assert_eq!(by, [0, 1]);
        let labels = l.names(&cam, &dots, &[]);
        assert_eq!(labels.len(), 3);
        // Something in the way hides a name.
        let (_, bx, by) = labels[0];
        let blocked = l.names(&cam, &dots, &[(bx, by - 5.0, bx + 20.0, by + 5.0)]);
        assert!(blocked.iter().all(|(i, _, _)| *i != 0));
        // Zoomed out: dots, no names.
        cam.alt = 2.0;
        assert!(l.names(&cam, &l.dots(&cam, None), &[]).is_empty());
        let d = dots[0];
        assert_eq!(l.hit(&dots, d.x + 3.0, d.y), Some(d.index));
        assert_eq!(l.hit(&dots, d.x + 30.0, d.y + 30.0), None);
    }

    #[test]
    fn search_entries_name_the_family() {
        let l = parse(TSV).unwrap();
        let p = l.places();
        assert_eq!(p[0].country, "Language · isolate");
        assert_eq!(p[1].country, "Language · Indo-European");
        assert!((p[3].lat + 8.5).abs() < 1e-9);
    }

    #[test]
    fn the_committed_file_parses() {
        let l = parse(include_str!("../assets/data/languages.tsv")).unwrap();
        assert!(l.list.len() > 8000, "{}", l.list.len());
        assert_eq!(l.families[0], "Atlantic-Congo");
        assert!(l
            .list
            .iter()
            .any(|x| x.name == "Basque" && x.family.is_none()));
        assert!(!l.families.iter().any(|f| f == "Bookkeeping"));
        assert_eq!(l.variables[0].id, ENDANGERMENT);
        assert_eq!(l.variables[0].codes.len(), ENDANGERMENT_COLORS.len());
        for v in &l.variables {
            assert!(v.codes.len() <= FAMILY_COLORS.len(), "{}", v.id);
            assert!(
                v.codes.iter().all(|(c, _)| v.color(*c).is_some()),
                "{}",
                v.id
            );
        }
        let mandarin = l.list.iter().position(|x| x.code == "mand1415").unwrap();
        let classifiers = l.variable("GB057").unwrap();
        assert_eq!(l.value(classifiers, mandarin), Some(1));
        // Most languages have an endangerment status; Grambank covers about 2,400.
        assert!(l.without_data(0) < 200);
        let gb = l.list.len() - l.without_data(classifiers);
        assert!((1500..3000).contains(&gb), "{gb}");
    }
}
