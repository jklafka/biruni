//! The societies layer: the Ethnographic Atlas societies (from D-PLACE) as
//! dots colored by one coded variable at a time. Browser-free. The data is
//! `assets/data/societies.tsv`, built by `scripts/convert_dplace.py`
//! (format described there).

use crate::camera::Camera;
use crate::languages::{place_names, screen_dots, Dot, FAMILY_COLORS, NAMES_BELOW_ALT};
use crate::stars::Rect;

pub const MAGIC: &str = "MAPSSOCIETIES1";
/// Color of a society with no value for the variable shown.
pub const NO_DATA_COLOR: &str = "#5c6270";

#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    pub id: String,
    pub title: String,
    /// (code, label), by code.
    pub codes: Vec<(u8, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Society {
    pub id: String,
    pub name: String,
    /// Radians.
    pub lat: f64,
    pub lon: f64,
    pub region: String,
    pub year: String,
    /// One code per variable; `None` when the Atlas has no value.
    pub values: Vec<Option<u8>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Societies {
    pub variables: Vec<Variable>,
    pub list: Vec<Society>,
}

/// Parse `societies.tsv`. Malformed lines are skipped; a wrong header or no
/// variables is an error.
pub fn parse(text: &str) -> Result<Societies, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(MAGIC) {
        return Err("not a societies file (bad header)".into());
    }
    let mut variables = Vec::new();
    let mut list = Vec::new();
    for line in lines {
        let f: Vec<&str> = line.split('\t').collect();
        match f.first() {
            Some(&"V") if f.len() == 4 => {
                let codes = f[3]
                    .split('|')
                    .filter_map(|p| {
                        let (n, label) = p.split_once('=')?;
                        Some((n.parse::<u8>().ok()?, label.to_string()))
                    })
                    .collect();
                variables.push(Variable {
                    id: f[1].to_string(),
                    title: f[2].to_string(),
                    codes,
                });
            }
            Some(&"S") if f.len() == 7 + variables.len() && !variables.is_empty() => {
                let (Ok(lat), Ok(lon)) = (f[3].parse::<f64>(), f[4].parse::<f64>()) else {
                    continue;
                };
                if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0)
                {
                    continue;
                }
                let values = f[7..]
                    .iter()
                    .zip(&variables)
                    .map(|(v, var)| {
                        let code = v.parse::<u8>().ok()?;
                        var.codes.iter().any(|(c, _)| *c == code).then_some(code)
                    })
                    .collect();
                list.push(Society {
                    id: f[1].to_string(),
                    name: f[2].to_string(),
                    lat: lat.to_radians(),
                    lon: lon.to_radians(),
                    region: f[5].to_string(),
                    year: f[6].to_string(),
                    values,
                });
            }
            _ => {}
        }
    }
    if variables.is_empty() {
        return Err("societies file without variables".into());
    }
    Ok(Societies { variables, list })
}

impl Societies {
    /// Color of a code of variable `v`: its position among the codes picks a
    /// color of the shared categorical palette.
    pub fn code_color(&self, v: usize, code: Option<u8>) -> &'static str {
        let Some(code) = code else {
            return NO_DATA_COLOR;
        };
        self.variables[v]
            .codes
            .iter()
            .position(|(c, _)| *c == code)
            .and_then(|i| FAMILY_COLORS.get(i).copied())
            .unwrap_or(NO_DATA_COLOR)
    }

    pub fn color(&self, v: usize, i: usize) -> &'static str {
        self.code_color(v, self.list[i].values.get(v).copied().flatten())
    }

    pub fn dots(&self, cam: &Camera) -> Vec<Dot> {
        screen_dots(cam, self.list.iter().map(|s| (s.lat, s.lon)))
    }

    pub fn names(&self, cam: &Camera, dots: &[Dot], avoid: &[Rect]) -> Vec<(usize, f64, f64)> {
        if cam.alt > NAMES_BELOW_ALT {
            return Vec::new();
        }
        place_names(cam, dots, |i| self.list[i].name.clone(), avoid)
    }

    /// The label of a society's value for variable `v`.
    pub fn value_label(&self, v: usize, i: usize) -> &str {
        let code = self.list[i].values.get(v).copied().flatten();
        code.and_then(|c| self.variables[v].codes.iter().find(|(n, _)| *n == c))
            .map_or("no data", |(_, l)| l.as_str())
    }

    /// HUD text: "!Kung (Southern Africa, 1950): Gathering".
    pub fn describe(&self, v: usize, i: usize) -> String {
        let s = &self.list[i];
        format!(
            "{} ({}, {}): {}",
            s.name,
            s.region,
            s.year,
            self.value_label(v, i)
        )
    }

    /// The legend of variable `v`: (color, "label (count)"), then no data.
    pub fn legend(&self, v: usize) -> Vec<(&'static str, String)> {
        let count = |code: Option<u8>| {
            self.list
                .iter()
                .filter(|s| s.values.get(v).copied().flatten() == code)
                .count()
        };
        let mut rows: Vec<(&'static str, String)> = self.variables[v]
            .codes
            .iter()
            .map(|(c, label)| {
                (
                    self.code_color(v, Some(*c)),
                    format!("{label} ({})", count(Some(*c))),
                )
            })
            .collect();
        rows.push((NO_DATA_COLOR, format!("No data ({})", count(None))));
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TSV: &str = "MAPSSOCIETIES1
V\tEA042\tSubsistence\t1=Gathering|2=Fishing|7=Intensive agriculture
V\tEA043\tDescent\t1=Patrilineal|6=Bilateral
S\tAa1\t!Kung\t-20.00\t21.00\tSouthern Africa\t1950\t1\t6
S\tAa2\tDorobo\t0.00\t36.00\tEast Tropical Africa\t1920\t2\t
S\tX\tBad code\t1\t1\tR\t1900\t9\t1
S\tY\tShort\t1\t1
S\tZ\tFar\t95\t1\tR\t1900\t1\t1
";

    #[test]
    fn parses_variables_and_values() {
        let s = parse(TSV).unwrap();
        assert_eq!(s.variables.len(), 2);
        assert_eq!(s.variables[0].codes[2], (7, "Intensive agriculture".into()));
        assert_eq!(s.list.len(), 3, "short and out-of-range rows skipped");
        assert_eq!(s.list[1].values, vec![Some(2), None]);
        assert_eq!(s.list[2].values[0], None, "an unknown code is no data");
        assert!(parse("MAPSSOCIETIES1\nS\ta\tb\t1\t1\tr\t1\n").is_err());
        assert!(parse("MAPSLANGS1\n").is_err());
    }

    #[test]
    fn colors_legend_and_text() {
        let s = parse(TSV).unwrap();
        assert_eq!(s.color(0, 0), FAMILY_COLORS[0]);
        assert_eq!(s.color(0, 1), FAMILY_COLORS[1]);
        assert_eq!(s.color(1, 1), NO_DATA_COLOR);
        assert_eq!(s.color(1, 0), FAMILY_COLORS[1], "Bilateral is the 2nd code");
        assert_eq!(s.describe(0, 0), "!Kung (Southern Africa, 1950): Gathering");
        assert_eq!(
            s.describe(1, 1),
            "Dorobo (East Tropical Africa, 1920): no data"
        );
        let legend = s.legend(0);
        assert_eq!(legend[0].1, "Gathering (1)");
        assert_eq!(legend[2].1, "Intensive agriculture (0)");
        assert_eq!(legend[3], (NO_DATA_COLOR, "No data (1)".into()));
    }

    #[test]
    fn the_committed_file_parses() {
        let s = parse(include_str!("../assets/data/societies.tsv")).unwrap();
        assert_eq!(s.variables.len(), 6);
        assert_eq!(s.list.len(), 1291);
        assert!(s
            .variables
            .iter()
            .all(|v| v.codes.len() <= FAMILY_COLORS.len()));
        let kung = s.list.iter().position(|x| x.name == "!Kung").unwrap();
        assert_eq!(s.value_label(0, kung), "Gathering");
    }
}
