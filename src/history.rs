//! Time-indexed vector data for the historical layers: the "HIST" format
//! (documented in `scripts/hist_format.py` and `docs/historical-data.md`).
//! Browser-free.
//!
//! A file holds names, geometries (rings, filled even-odd) and records; a
//! record says that a name had a geometry from one time to another
//! (inclusive). Drawing a moment means picking the records that hold then
//! ([`Timeline::at`]), so no per-year snapshots are stored. Two time scales
//! share the rule "later is larger": calendar years (-44 is 44 BCE; there
//! is no year 0), and geological time as -(age in tenths of a million
//! years).

// The historical layers use this from bead maps-3sn.4 on.
#![cfg_attr(not(test), allow(dead_code))]

use crate::geo::{read_delta, read_varint, Shape};

/// What the time values of a file count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    /// Calendar years, negative BCE, no year 0.
    Years,
    /// -(age in tenths of a million years).
    TenthsMa,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub name: u32,
    /// First and last time the record holds (inclusive).
    pub from: i64,
    pub to: i64,
    pub geometry: u32,
    /// Where the name goes, degrees (lon, lat).
    pub label: [f64; 2],
    /// A plate id for geological data; 0 when unused.
    pub tag: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Timeline {
    pub scale: Scale,
    pub names: Vec<String>,
    /// Each geometry is a set of closed rings, filled even-odd.
    pub geometries: Vec<Vec<Shape>>,
    pub records: Vec<Record>,
}

impl Timeline {
    /// The records that hold at time `t`, in file order.
    pub fn at(&self, t: i64) -> impl Iterator<Item = &Record> {
        self.records
            .iter()
            .filter(move |r| r.from <= t && t <= r.to)
    }

    /// The span covered: the earliest start and the latest end.
    pub fn span(&self) -> Option<(i64, i64)> {
        let from = self.records.iter().map(|r| r.from).min()?;
        let to = self.records.iter().map(|r| r.to).max()?;
        Some((from, to))
    }

    pub fn name(&self, r: &Record) -> &str {
        &self.names[r.name as usize]
    }
}

fn read_zigzag(data: &[u8], pos: &mut usize) -> Result<i64, String> {
    read_delta(data, pos)
}

/// A count that cannot exceed the bytes left (every item takes one).
fn read_count(data: &[u8], pos: &mut usize) -> Result<usize, String> {
    let n = read_varint(data, pos)?;
    if n > (data.len() - *pos) as u64 {
        return Err("history data: count exceeds data size".into());
    }
    Ok(n as usize)
}

/// Decode a HIST file. Rejects malformed input (bad header, truncation,
/// trailing bytes, out-of-range coordinates or indices) without panicking.
pub fn decode(data: &[u8]) -> Result<Timeline, String> {
    if data.len() < 8 || &data[..4] != b"HIST" {
        return Err("not a history data file".into());
    }
    if data[4] != 1 {
        return Err(format!("unsupported history data version {}", data[4]));
    }
    let scale = match data[5] {
        0 => Scale::Years,
        1 => Scale::TenthsMa,
        s => return Err(format!("history data: unknown time scale {s}")),
    };
    let digits = data[6];
    if digits > 6 {
        return Err("history data: bad coordinate digits".into());
    }
    let k = 10f64.powi(digits as i32);
    let (max_x, max_y) = (
        180 * 10i64.pow(digits as u32),
        90 * 10i64.pow(digits as u32),
    );
    let mut pos = 8;
    let mut names = Vec::new();
    for _ in 0..read_count(data, &mut pos)? {
        let n = read_count(data, &mut pos)?;
        let s = std::str::from_utf8(&data[pos..pos + n]).map_err(|_| "history data: bad name")?;
        names.push(s.to_string());
        pos += n;
    }
    let (mut x, mut y) = (0i64, 0i64);
    let mut geometries = Vec::new();
    for _ in 0..read_count(data, &mut pos)? {
        let mut rings = Vec::new();
        for _ in 0..read_count(data, &mut pos)? {
            let n = read_count(data, &mut pos)?;
            if n < 3 {
                return Err("history data: ring with fewer than 3 points".into());
            }
            let mut points = Vec::with_capacity(n);
            for _ in 0..n {
                x = x
                    .checked_add(read_zigzag(data, &mut pos)?)
                    .ok_or("coordinate overflow")?;
                y = y
                    .checked_add(read_zigzag(data, &mut pos)?)
                    .ok_or("coordinate overflow")?;
                if x.abs() > max_x || y.abs() > max_y {
                    return Err("history data: coordinate out of range".into());
                }
                points.push([x as f64 / k, y as f64 / k]);
            }
            rings.push(Shape {
                points,
                closed: true,
            });
        }
        geometries.push(rings);
    }
    let mut records = Vec::new();
    for _ in 0..read_count(data, &mut pos)? {
        let name = read_varint(data, &mut pos)?;
        let from = read_zigzag(data, &mut pos)?;
        let to = read_zigzag(data, &mut pos)?;
        let geometry = read_varint(data, &mut pos)?;
        let lx = read_zigzag(data, &mut pos)?;
        let ly = read_zigzag(data, &mut pos)?;
        let tag = read_varint(data, &mut pos)?;
        if name >= names.len() as u64 || geometry >= geometries.len() as u64 || from > to {
            return Err("history data: bad record".into());
        }
        if lx.abs() > max_x || ly.abs() > max_y || tag > u32::MAX as u64 {
            return Err("history data: bad label or tag".into());
        }
        records.push(Record {
            name: name as u32,
            from,
            to,
            geometry: geometry as u32,
            label: [lx as f64 / k, ly as f64 / k],
            tag: tag as u32,
        });
    }
    if pos != data.len() {
        return Err("history data: trailing bytes".into());
    }
    Ok(Timeline {
        scale,
        names,
        geometries,
        records,
    })
}

/// A calendar year for people: "44 BCE", "1914". (Year values follow the
/// file convention: negative BCE, no year 0.)
pub fn format_year(year: i64) -> String {
    if year < 0 {
        format!("{} BCE", -year)
    } else {
        year.to_string()
    }
}

/// The year `n` years after `year`, skipping the missing year 0 (1 BCE is
/// followed by 1 CE).
pub fn add_years(year: i64, n: i64) -> i64 {
    // Astronomical numbering (1 BCE = 0) has no gap; convert there and back.
    let astro = if year < 0 { year + 1 } else { year };
    let moved = astro + n;
    if moved <= 0 {
        moved - 1
    } else {
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Timeline {
        decode(include_bytes!("../scripts/testdata/tiny_history.bin")).unwrap()
    }

    #[test]
    fn the_fixture_decodes() {
        let t = fixture();
        assert_eq!(t.scale, Scale::Years);
        assert_eq!(t.names, ["Squareland", "Ost", "Ünïcode"]);
        assert_eq!(t.geometries.len(), 3);
        assert_eq!(t.geometries[0].len(), 2, "outer ring and hole");
        assert_eq!(t.geometries[1][2].points[0], [-20.0, 0.0]);
        assert_eq!(t.geometries[2][0].points[1], [180.0, -5.0]);
        assert!(t.geometries.iter().flatten().all(|r| r.closed));
        assert_eq!(t.records[2].label, [175.0, 0.0]);
        assert_eq!(t.records[2].tag, 7);
        assert_eq!(t.span(), Some((-44, 2024)));
    }

    #[test]
    fn records_hold_for_their_years_inclusive() {
        let t = fixture();
        let names = |y: i64| t.at(y).map(|r| t.name(r).to_string()).collect::<Vec<_>>();
        assert!(names(-45).is_empty());
        assert_eq!(names(-44), ["Squareland"]);
        assert_eq!(names(-1), ["Squareland", "Ünïcode"]);
        assert_eq!(names(99), ["Squareland"]);
        // Squareland changes shape at 100.
        let g = |y: i64| t.at(y).find(|r| r.name == 0).map(|r| r.geometry);
        assert_eq!(
            (g(99), g(100), g(199), g(200)),
            (Some(0), Some(1), Some(1), None)
        );
        assert_eq!(names(150), ["Squareland", "Ost"]);
        assert_eq!(names(2024), ["Ost"]);
        assert!(names(2025).is_empty());
    }

    #[test]
    fn malformed_data_is_refused() {
        let good = include_bytes!("../scripts/testdata/tiny_history.bin");
        assert!(decode(&good[..good.len() - 1]).is_err());
        let mut more = good.to_vec();
        more.push(0);
        assert!(decode(&more).is_err());
        for (i, v) in [(4, 2u8), (5, 9), (6, 9)] {
            let mut bad = good.to_vec();
            bad[i] = v;
            assert!(decode(&bad).is_err(), "byte {i}");
        }
        assert!(decode(b"MAPD\x01").is_err());
        assert!(decode(b"").is_err());
        // Every truncation fails cleanly.
        for n in 0..good.len() {
            assert!(decode(&good[..n]).is_err(), "{n}");
        }
    }

    #[test]
    fn years_read_and_count_without_a_year_zero() {
        assert_eq!(format_year(-44), "44 BCE");
        assert_eq!(format_year(1914), "1914");
        assert_eq!(add_years(-1, 1), 1);
        assert_eq!(add_years(1, -1), -1);
        assert_eq!(add_years(-44, 44), 1);
        assert_eq!(add_years(-44, 43), -1);
        assert_eq!(add_years(1900, 124), 2024);
        assert_eq!(add_years(10, -20), -11);
    }
}
