//! The languages layer: Glottolog's languages as dots colored by family,
//! with names when zoomed in. Browser-free (the overlay draws what this
//! returns). The data is `assets/data/languages.tsv`, built by
//! `scripts/convert_glottolog.py` (format described there).

use crate::camera::Camera;
use crate::labels::facing;
use crate::stars::{overlaps, Rect};

/// First line of `languages.tsv`.
pub const MAGIC: &str = "MAPSLANGS1";
/// The largest families get their own color, in order of size; the rest
/// share [`OTHER_COLOR`], and isolates have [`ISOLATE_COLOR`].
pub const FAMILY_COLORS: [&str; 12] = [
    "#e6194b", "#3cb44b", "#ffe119", "#4363d8", "#f58231", "#42d4f4", "#f032e6", "#bfef45",
    "#fabed4", "#469990", "#dcbeff", "#9a6324",
];
pub const OTHER_COLOR: &str = "#a9a9a9";
pub const ISOLATE_COLOR: &str = "#ffffff";
/// Dot radius and the hit radius around it, CSS pixels.
pub const DOT_PX: f64 = 2.6;
pub const HIT_PX: f64 = 7.0;
/// Names show below this altitude (globe radii, about 2,500 km), at most
/// [`MAX_NAMES`] at once.
pub const NAMES_BELOW_ALT: f64 = 0.4;
pub const MAX_NAMES: usize = 80;

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
}

#[derive(Clone, Debug, PartialEq)]
pub struct Languages {
    pub list: Vec<Language>,
    /// Family names, largest first (by number of languages), ties by name.
    pub families: Vec<String>,
    pub family_sizes: Vec<usize>,
    pub isolates: usize,
}

/// Parse `languages.tsv`. Malformed lines are skipped; a wrong header is an
/// error.
pub fn parse(text: &str) -> Result<Languages, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(MAGIC) {
        return Err("not a languages file (bad header)".into());
    }
    let mut raw = Vec::new();
    let mut counts: std::collections::HashMap<&str, usize> = Default::default();
    for line in lines {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 7 || f[1].is_empty() {
            continue;
        }
        let (Ok(lat), Ok(lon)) = (f[3].parse::<f64>(), f[4].parse::<f64>()) else {
            continue;
        };
        if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0) {
            continue;
        }
        if !f[2].is_empty() {
            *counts.entry(f[2]).or_default() += 1;
        }
        raw.push((f, lat, lon));
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
            code: f[0].to_string(),
            name: f[1].to_string(),
            family: index.get(f[2]).copied(),
            lat: lat.to_radians(),
            lon: lon.to_radians(),
            iso: f[5].to_string(),
            area: f[6].to_string(),
        })
        .collect();
    let isolates = list.iter().filter(|l| l.family.is_none()).count();
    Ok(Languages {
        list,
        families: fams.iter().map(|(n, _)| n.to_string()).collect(),
        family_sizes: fams.iter().map(|(_, c)| *c).collect(),
        isolates,
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
    /// CSS color of a language's dot.
    pub fn color(&self, i: usize) -> &'static str {
        match self.list[i].family {
            None => ISOLATE_COLOR,
            Some(f) => FAMILY_COLORS
                .get(f as usize)
                .copied()
                .unwrap_or(OTHER_COLOR),
        }
    }

    pub fn family_name(&self, i: usize) -> &str {
        match self.list[i].family {
            None => "isolate",
            Some(f) => &self.families[f as usize],
        }
    }

    /// The languages on screen and on the near side of the globe.
    pub fn dots(&self, cam: &Camera) -> Vec<Dot> {
        screen_dots(cam, self.list.iter().map(|l| (l.lat, l.lon)))
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

    /// HUD text: "Basque (isolate, Eurasia)".
    pub fn describe(&self, i: usize) -> String {
        let l = &self.list[i];
        let mut parts = vec![self.family_name(i).to_string()];
        if !l.area.is_empty() {
            parts.push(l.area.replace(';', ", "));
        }
        format!("{} ({})", l.name, parts.join(", "))
    }

    /// The legend: (color, label) for the colored families, then the rest
    /// and the isolates.
    pub fn legend(&self) -> Vec<(&'static str, String)> {
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

    const TSV: &str = "MAPSLANGS1
basq1248\tBasque\t\t43.2787\t-1.3164\teus\tEurasia
stan1293\tEnglish\tIndo-European\t53.0\t-1.0\teng\tEurasia
stan1288\tSpanish\tIndo-European\t40.4414\t-1.1170\tspa\tEurasia
swah1253\tSwahili\tAtlantic-Congo\t-8.5\t37.0\tswh\tAfrica
bad\t\tX\t1\t1\t\tAfrica
nan1\tBroken\tX\tnorth\t1\t\tAfrica
far1\tFar\tX\t95\t1\t\tAfrica
short\tShort\tX
";

    #[test]
    fn parses_and_ranks_families() {
        let l = parse(TSV).unwrap();
        assert_eq!(l.list.len(), 4, "bad rows skipped");
        assert_eq!(l.families, ["Indo-European", "Atlantic-Congo"]);
        assert_eq!(l.family_sizes, [2, 1]);
        assert_eq!(l.isolates, 1);
        assert_eq!(l.color(0), ISOLATE_COLOR);
        assert_eq!(l.color(1), FAMILY_COLORS[0]);
        assert_eq!(l.color(3), FAMILY_COLORS[1]);
        assert_eq!(l.describe(0), "Basque (isolate, Eurasia)");
        assert_eq!(l.describe(3), "Swahili (Atlantic-Congo, Africa)");
        assert!(parse("MAPSPLACES1\n").is_err());
        let legend = l.legend();
        assert_eq!(
            legend[0],
            (FAMILY_COLORS[0], "Indo-European (2)".to_string())
        );
        assert_eq!(legend.last().unwrap().1, "Isolates (1)");
    }

    #[test]
    fn dots_names_and_hits_on_screen() {
        let l = parse(TSV).unwrap();
        let mut cam = Camera::new(47.0, 0.0, 0.39);
        cam.width = 800.0;
        cam.height = 600.0;
        let dots = l.dots(&cam);
        let names: Vec<&str> = dots.iter().map(|d| l.list[d.index].name.as_str()).collect();
        assert_eq!(
            names,
            ["Basque", "English", "Spanish"],
            "Swahili is off screen"
        );
        let labels = l.names(&cam, &dots, &[]);
        assert_eq!(labels.len(), 3);
        // Something in the way hides a name.
        let (_, bx, by) = labels[0];
        let blocked = l.names(&cam, &dots, &[(bx, by - 5.0, bx + 20.0, by + 5.0)]);
        assert!(blocked.iter().all(|(i, _, _)| *i != 0));
        // Zoomed out: dots, no names.
        cam.alt = 2.0;
        assert!(l.names(&cam, &l.dots(&cam), &[]).is_empty());
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
    }
}
