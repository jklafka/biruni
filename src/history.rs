//! Time-indexed vector data for the historical layers: the "HIST" format
//! (documented in `scripts/hist_format.py` and `docs/historical-data.md`).
//! Browser-free.
//!
//! A file holds names, geometries (rings, filled even-odd) and records; a
//! record says that a name had a geometry from one time to another
//! (inclusive). Drawing a moment means picking the records that hold then
//! ([`Timeline::active`]), so no per-year snapshots are stored. Two time scales
//! share the rule "later is larger": calendar years (-44 is 44 BCE; there
//! is no year 0), and geological time as -(age in tenths of a million
//! years).

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
    /// Closed rings, shared between geometries.
    pub rings: Vec<Shape>,
    /// Each geometry is a set of rings (indices into `rings`), filled
    /// even-odd.
    pub geometries: Vec<Vec<u32>>,
    pub records: Vec<Record>,
}

impl Timeline {
    /// The records that hold at time `t`, in file order.
    #[cfg(test)]
    pub fn at(&self, t: i64) -> impl Iterator<Item = &Record> {
        self.records
            .iter()
            .filter(move |r| r.from <= t && t <= r.to)
    }

    /// The span covered: the earliest start and the latest end.
    #[cfg(test)]
    pub fn span(&self) -> Option<(i64, i64)> {
        let from = self.records.iter().map(|r| r.from).min()?;
        let to = self.records.iter().map(|r| r.to).max()?;
        Some((from, to))
    }

    pub fn name(&self, r: &Record) -> &str {
        &self.names[r.name as usize]
    }

    /// The rings of a record's geometry.
    pub fn rings_of<'a>(&'a self, r: &Record) -> impl Iterator<Item = &'a Shape> + 'a {
        self.geometries[r.geometry as usize]
            .iter()
            .map(|&i| &self.rings[i as usize])
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
    let mut rings: Vec<Shape> = Vec::new();
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
    let mut geometries = Vec::new();
    for _ in 0..read_count(data, &mut pos)? {
        let mut refs = Vec::new();
        for _ in 0..read_count(data, &mut pos)? {
            let i = read_varint(data, &mut pos)?;
            if i >= rings.len() as u64 {
                return Err("history data: bad ring index".into());
            }
            refs.push(i as u32);
        }
        geometries.push(refs);
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
        rings,
        geometries,
        records,
    })
}

/// The years the borders layer covers (Cliopatria: 3400 BCE to 2024).
pub const YEAR_MIN: i64 = -3400;
pub const YEAR_MAX: i64 = 2024;
/// The year the borders layer opens at.
pub const DEFAULT_YEAR: i64 = 1500;

/// Clamp a year into the covered span; year 0 (which does not exist)
/// becomes 1 CE.
pub fn clamp_year(year: i64) -> i64 {
    match year.clamp(YEAR_MIN, YEAR_MAX) {
        0 => 1,
        y => y,
    }
}

/// `&year=<n>` from a URL hash (negative for BCE): the historical borders
/// are shown at that year. `None` if absent or malformed.
pub fn parse_hash_year(hash: &str) -> Option<i64> {
    let v = hash
        .trim_start_matches('#')
        .split('&')
        .skip(1)
        .find_map(|kv| kv.strip_prefix("year="))?;
    v.parse::<i64>().ok().map(clamp_year)
}

pub fn format_hash_year(year: i64) -> String {
    format!("&year={year}")
}

/// Fill color index of a borders record (tag bits 0-2, see
/// `scripts/convert_cliopatria.py`), below `PALETTE_SIZE`.
pub fn color_of(r: &Record) -> u8 {
    (r.tag & 7) as u8 % crate::country::PALETTE_SIZE
}

/// Label tier of a borders record (tag bits 3-5; 0 most important).
pub fn tier_of(r: &Record) -> u8 {
    ((r.tag >> 3) & 7) as u8
}

impl Timeline {
    /// Indices of the records that hold at `t`, at most 255 (the ID raster
    /// is 8-bit); ID `i + 1` in the raster is `active[i]`.
    pub fn active(&self, t: i64) -> Vec<usize> {
        let mut v = self.holding(t);
        v.truncate(255);
        v
    }

    /// Indices of all the records that hold at `t`.
    pub fn holding(&self, t: i64) -> Vec<usize> {
        self.records
            .iter()
            .enumerate()
            .filter(|(_, r)| r.from <= t && t <= r.to)
            .map(|(i, _)| i)
            .collect()
    }

    /// Rasterize the active records into a `width x width/2` equirectangular
    /// grid of IDs (`position + 1`, 0 for none), row 0 at the north, each
    /// texel taking the record that contains its center (even-odd over the
    /// record's rings; later records win where they overlap). The same
    /// layout as `Countries::rasterize_ids`, so the political shader reads
    /// it unchanged.
    pub fn rasterize(&self, active: &[usize], width: usize) -> Vec<u8> {
        let height = width / 2;
        let mut out = vec![0u8; width * height];
        if height == 0 {
            return out;
        }
        let (dx, dy) = (360.0 / width as f64, 180.0 / height as f64);
        let mut xs: Vec<f64> = Vec::new();
        for (pos, &ri) in active.iter().enumerate() {
            let rec = &self.records[ri];
            let id = (pos + 1) as u8;
            let rings: Vec<&Shape> = self.rings_of(rec).collect();
            let (lo, hi) = rings
                .iter()
                .flat_map(|r| &r.points)
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                    (lo.min(p[1]), hi.max(p[1]))
                });
            if !lo.is_finite() {
                continue;
            }
            let r0 = ((90.0 - hi) / dy - 0.5).ceil().max(0.0) as usize;
            let r1 = ((90.0 - lo) / dy - 0.5).floor();
            if r1 < 0.0 {
                continue;
            }
            for row in r0..=(r1 as usize).min(height - 1) {
                let lat = 90.0 - (row as f64 + 0.5) * dy;
                xs.clear();
                for ring in &rings {
                    let p = &ring.points;
                    let mut j = p.len() - 1;
                    for i in 0..p.len() {
                        let (a, b) = (p[i], p[j]);
                        if (a[1] > lat) != (b[1] > lat) {
                            xs.push(a[0] + (lat - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                        }
                        j = i;
                    }
                }
                xs.sort_by(|a, b| a.total_cmp(b));
                for pair in xs.as_chunks::<2>().0 {
                    let c0 = ((pair[0] + 180.0) / dx - 0.5).ceil().max(0.0) as usize;
                    let c1 = (((pair[1] + 180.0) / dx - 0.5).ceil().max(0.0) as usize).min(width);
                    if c0 < c1 {
                        out[row * width + c0..row * width + c1].fill(id);
                    }
                }
            }
        }
        out
    }

    /// Fill color index of each raster ID (index 0 unused), for
    /// `Renderer::set_country_colors`.
    pub fn id_colors(&self, active: &[usize]) -> [u8; 256] {
        let mut out = [0u8; 256];
        for (pos, &ri) in active.iter().enumerate().take(255) {
            out[pos + 1] = color_of(&self.records[ri]);
        }
        out
    }

    /// The rings of the active records, each once, for the border lines.
    pub fn border_rings(&self, active: &[usize]) -> Vec<Shape> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for &ri in active {
            for &g in &self.geometries[self.records[ri].geometry as usize] {
                if seen.insert(g) {
                    out.push(self.rings[g as usize].clone());
                }
            }
        }
        out
    }

    /// Name labels of the active records, as country labels.
    pub fn labels(&self, active: &[usize]) -> Vec<crate::labels::Label> {
        active
            .iter()
            .map(|&ri| {
                let r = &self.records[ri];
                // Angular size: the bounding box of the rings around the
                // label point.
                let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
                for p in self.rings_of(r).flat_map(|s| &s.points) {
                    x0 = x0.min(p[0]);
                    x1 = x1.max(p[0]);
                    y0 = y0.min(p[1]);
                    y1 = y1.max(p[1]);
                }
                let w = (x1 - x0) * r.label[1].to_radians().cos();
                crate::labels::Label {
                    kind: crate::labels::Kind::Country,
                    text: self.name(r).to_uppercase(),
                    lat: r.label[1].to_radians(),
                    lon: r.label[0].to_radians(),
                    tier: tier_of(r),
                    population: 0,
                    extent: w.max(y1 - y0).max(0.0).to_radians(),
                    width: 0.0,
                }
            })
            .collect()
    }

    /// The active record whose raster ID is at a lon/lat (degrees), using
    /// the raster `ids` of `width` from [`Timeline::rasterize`].
    pub fn record_at(
        &self,
        active: &[usize],
        ids: &[u8],
        width: usize,
        lat: f64,
        lon: f64,
    ) -> Option<usize> {
        let height = width / 2;
        if height == 0 || !(lat.is_finite() && lon.is_finite()) {
            return None;
        }
        let lon = (lon + 180.0).rem_euclid(360.0) - 180.0;
        let c = (((lon + 180.0) / 360.0 * width as f64) as usize).min(width - 1);
        let r = (((90.0 - lat) / 180.0 * height as f64) as usize).min(height - 1);
        let id = *ids.get(r * width + c)? as usize;
        (id > 0).then(|| active.get(id - 1).copied()).flatten()
    }

    /// HUD text for a record: its name and years.
    pub fn describe(&self, ri: usize) -> Option<String> {
        let r = self.records.get(ri)?;
        Some(format!(
            "{} ({} to {})",
            self.name(r),
            format_year(r.from),
            format_year(r.to)
        ))
    }
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
        assert_eq!(t.rings.len(), 4, "shared rings stored once");
        assert_eq!(t.geometries[0].len(), 2, "outer ring and hole");
        assert_eq!(t.geometries[1][..2], t.geometries[0][..]);
        let ring = |g: usize, i: usize| &t.rings[t.geometries[g][i] as usize];
        assert_eq!(ring(1, 2).points[0], [-20.0, 0.0]);
        assert_eq!(ring(2, 0).points[1], [180.0, -5.0]);
        assert!(t.rings.iter().all(|r| r.closed));
        assert_eq!(t.rings_of(&t.records[1]).count(), 3);
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
    fn the_raster_fills_the_records_of_the_year() {
        let t = fixture();
        let active = t.active(150); // Squareland (with the western square) and Ost
        assert_eq!(active, vec![1, 2]);
        let w = 360; // one texel per degree
        let ids = t.rasterize(&active, w);
        let at = |lon: f64, lat: f64| t.record_at(&active, &ids, w, lat, lon);
        assert_eq!(at(2.0, 2.0), Some(1), "inside the square");
        assert_eq!(at(5.0, 5.0), None, "in the hole");
        assert_eq!(at(-15.0, 5.0), Some(1), "the added western square");
        assert_eq!(at(175.0, 0.0), Some(2), "Ost, at the seam");
        assert_eq!(at(-175.0, 0.0), None);
        assert_eq!(at(50.0, 50.0), None);
        // Before 100, Squareland has no western square.
        let a2 = t.active(50);
        let ids2 = t.rasterize(&a2, w);
        assert_eq!(t.record_at(&a2, &ids2, w, 5.0, -15.0), None);
        // Colors and tiers from the tag; Ost's tag is 7, so color 7 % 6.
        let colors = t.id_colors(&active);
        assert_eq!(colors[2], 1);
        assert_eq!(tier_of(&t.records[2]), 0);
        // Border rings: square, hole, western square, seam square.
        assert_eq!(t.border_rings(&active).len(), 4);
        let labels = t.labels(&active);
        assert_eq!(labels[1].text, "OST");
        assert!((labels[1].lon.to_degrees() - 175.0).abs() < 1e-9);
        assert!(labels[0].extent > 0.3, "{}", labels[0].extent);
        assert_eq!(t.describe(2).unwrap(), "Ost (150 to 2024)");
        assert_eq!(t.describe(0).unwrap(), "Squareland (44 BCE to 99)");
    }

    #[test]
    fn the_year_comes_from_the_hash() {
        assert_eq!(parse_hash_year("#1,2,3&year=1914"), Some(1914));
        assert_eq!(parse_hash_year("#1,2,3&layer=mono&year=-44"), Some(-44));
        assert_eq!(parse_hash_year("#1,2,3&year=-9999"), Some(YEAR_MIN));
        assert_eq!(parse_hash_year("#1,2,3&year=3000"), Some(YEAR_MAX));
        assert_eq!(parse_hash_year("#1,2,3&year=0"), Some(1));
        assert_eq!(parse_hash_year("#1,2,3&year=x"), None);
        assert_eq!(parse_hash_year("#1,2,3"), None);
        assert_eq!(format_hash_year(-44), "&year=-44");
    }

    #[test]
    fn the_real_borders_decode_and_fit_the_raster() {
        let t = decode(include_bytes!("../assets/data/history_borders.bin")).unwrap();
        assert_eq!(t.scale, Scale::Years);
        assert_eq!(t.span(), Some((YEAR_MIN, YEAR_MAX)));
        for year in [-3000, -44, 1, 800, 1500, 1914, 2024] {
            let a = t.active(year);
            assert!(!a.is_empty() && a.len() < 255, "{year}: {}", a.len());
        }
        let names = |y: i64| {
            t.active(y)
                .into_iter()
                .map(|i| t.name(&t.records[i]).to_string())
                .collect::<Vec<_>>()
        };
        assert!(
            names(117).iter().any(|n| n.contains("Roman")),
            "{:?}",
            names(117)
        );
        assert!(t
            .records
            .iter()
            .all(|r| color_of(r) < crate::country::PALETTE_SIZE));
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
