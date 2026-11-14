//! Country polygons and attributes: hit-testing and the ID raster.
//!
//! Everything here is browser-free so it can be tested natively; only
//! [`load`] touches the network, through `geo::fetch_bytes`.
//!
//! The data comes from Natural Earth admin-0 countries (public domain),
//! converted by `scripts/convert_geojson.py` into this binary format (all
//! integers are LEB128 varints):
//!
//! ```text
//! "MAPC" | version u8 = 1 | scale | country_count
//! country_count × (
//!     name | iso2 | iso3 | continent | subregion   (strings: len, UTF-8 bytes)
//!     population | polygon_count
//!     polygon_count × ( ring_count | ring_count × ( point_count | points ) )
//! )
//! ```
//!
//! `points` are zigzag `(dlon, dlat)` deltas in 1/`scale` degree from the
//! previous point of the file. Rings are implicitly closed (the repeated
//! first point is dropped). The first ring of a polygon is its outer
//! boundary and the rest are holes, but containment uses the even-odd rule
//! over all rings of a polygon, so ring order does not matter.
//!
//! Natural Earth splits countries that cross the antimeridian (Russia, Fiji,
//! Antarctica) into polygons that end at lon ±180, so planar lon/lat
//! point-in-polygon works without special seam handling. Hit-testing
//! normalizes the query longitude into [-180, 180].

use crate::geo::{read_delta, read_varint};
use wasm_bindgen::JsValue;

const MAGIC: &[u8; 4] = b"MAPC";
const VERSION: u8 = 1;
/// IDs in the raster are `index + 1` in a byte, with 0 meaning no country.
pub const MAX_COUNTRIES: usize = 255;

/// Bounding box as `[min_lon, min_lat, max_lon, max_lat]`.
type BBox = [f64; 4];

#[derive(Debug)]
struct Ring {
    points: Vec<[f64; 2]>,
    bbox: BBox,
}

#[derive(Debug)]
struct Polygon {
    rings: Vec<Ring>,
    bbox: BBox,
}

#[derive(Debug)]
pub struct Country {
    pub name: String,
    /// ISO 3166-1 alpha-2, empty when Natural Earth has none (e.g. Somaliland).
    pub iso2: String,
    pub iso3: String,
    pub continent: String,
    pub subregion: String,
    pub population: u64,
    polygons: Vec<Polygon>,
}

#[derive(Debug, Default)]
pub struct Countries {
    list: Vec<Country>,
    /// Polygon bounding boxes with their owner, scanned before the exact test.
    index: Vec<(BBox, usize, usize)>,
}

fn bbox_of(points: &[[f64; 2]]) -> BBox {
    points.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        },
    )
}

fn merge_bbox(a: BBox, b: BBox) -> BBox {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn read_string(data: &[u8], pos: &mut usize) -> Result<String, String> {
    let len = read_varint(data, pos)?;
    if len > (data.len() - *pos) as u64 {
        return Err("string exceeds data size".into());
    }
    let end = *pos + len as usize;
    let s = std::str::from_utf8(&data[*pos..end]).map_err(|_| "invalid UTF-8")?;
    *pos = end;
    Ok(s.to_owned())
}

/// Decode the country file. Rejects malformed input (bad header, truncation,
/// trailing bytes, out-of-range coordinates, oversized counts) without
/// panicking or allocating unbounded memory.
pub fn decode_countries(data: &[u8]) -> Result<Countries, String> {
    if data.len() < 5 || &data[..4] != MAGIC {
        return Err("not a country data file".into());
    }
    if data[4] != VERSION {
        return Err(format!("unsupported country data version {}", data[4]));
    }
    let mut pos = 5;
    let scale = read_varint(data, &mut pos)?;
    if scale == 0 || scale > 1_000_000 {
        return Err("invalid coordinate scale".into());
    }
    let (max_x, max_y) = (180 * scale as i64, 90 * scale as i64);
    let count = read_varint(data, &mut pos)?;
    if count > MAX_COUNTRIES as u64 {
        return Err("too many countries".into());
    }
    let mut list = Vec::with_capacity(count as usize);
    let (mut x, mut y) = (0i64, 0i64);
    for _ in 0..count {
        let name = read_string(data, &mut pos)?;
        let iso2 = read_string(data, &mut pos)?;
        let iso3 = read_string(data, &mut pos)?;
        let continent = read_string(data, &mut pos)?;
        let subregion = read_string(data, &mut pos)?;
        let population = read_varint(data, &mut pos)?;
        let n_polys = read_varint(data, &mut pos)?;
        // A polygon needs at least a ring count, a point count and 3 points.
        if n_polys > (data.len() - pos) as u64 / 8 {
            return Err("polygon count exceeds data size".into());
        }
        let mut polygons = Vec::with_capacity(n_polys as usize);
        for _ in 0..n_polys {
            let n_rings = read_varint(data, &mut pos)?;
            if n_rings == 0 || n_rings > (data.len() - pos) as u64 / 7 {
                return Err("invalid ring count".into());
            }
            let mut rings = Vec::with_capacity(n_rings as usize);
            for _ in 0..n_rings {
                let n = read_varint(data, &mut pos)?;
                if n < 3 || n > ((data.len() - pos) / 2) as u64 {
                    return Err("invalid point count".into());
                }
                let mut points = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    x = x
                        .checked_add(read_delta(data, &mut pos)?)
                        .ok_or("coordinate overflow")?;
                    y = y
                        .checked_add(read_delta(data, &mut pos)?)
                        .ok_or("coordinate overflow")?;
                    if x.abs() > max_x || y.abs() > max_y {
                        return Err("coordinate out of lon/lat range".into());
                    }
                    points.push([x as f64 / scale as f64, y as f64 / scale as f64]);
                }
                let bbox = bbox_of(&points);
                rings.push(Ring { points, bbox });
            }
            let bbox = rings
                .iter()
                .skip(1)
                .fold(rings[0].bbox, |b, r| merge_bbox(b, r.bbox));
            polygons.push(Polygon { rings, bbox });
        }
        list.push(Country {
            name,
            iso2,
            iso3,
            continent,
            subregion,
            population,
            polygons,
        });
    }
    if pos != data.len() {
        return Err("trailing bytes after last country".into());
    }
    let index = list
        .iter()
        .enumerate()
        .flat_map(|(ci, c)| {
            c.polygons
                .iter()
                .enumerate()
                .map(move |(pi, p)| (p.bbox, ci, pi))
        })
        .collect();
    Ok(Countries { list, index })
}

/// Even-odd crossing test of a point against one implicitly closed ring:
/// true if a ray from the point toward +lon crosses the ring an odd number of
/// times. XOR this across all rings of a polygon.
fn ring_parity(ring: &Ring, lon: f64, lat: f64) -> bool {
    let p = &ring.points;
    let mut inside = false;
    let mut j = p.len() - 1;
    for i in 0..p.len() {
        let (a, b) = (p[i], p[j]);
        if (a[1] > lat) != (b[1] > lat) {
            let x = a[0] + (lat - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if lon < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

fn polygon_contains(poly: &Polygon, lon: f64, lat: f64) -> bool {
    poly.rings
        .iter()
        .filter(|r| r.bbox[1] <= lat && lat <= r.bbox[3] && lon <= r.bbox[2])
        .fold(false, |acc, r| acc ^ ring_parity(r, lon, lat))
}

/// Normalize a longitude in degrees to [-180, 180].
fn wrap_lon(lon: f64) -> f64 {
    if (-180.0..=180.0).contains(&lon) {
        return lon;
    }
    let w = (lon + 180.0).rem_euclid(360.0) - 180.0;
    // Keep +180 as +180 rather than folding it to -180.
    if w == -180.0 && lon > 0.0 {
        180.0
    } else {
        w
    }
}

impl Countries {
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn get(&self, index: usize) -> Option<&Country> {
        self.list.get(index)
    }

    /// Index of the country containing a point, in degrees. Longitudes are
    /// wrapped into [-180, 180] (+180 counts as just inside the east edge so
    /// polygons ending at the seam match) and the latitude is clamped to
    /// ±89.998°, because Natural Earth's Antarctica ends at -89.9989°; this
    /// makes the geographic south pole resolve to Antarctica. Non-finite
    /// input gives `None`. Bounding boxes prefilter the exact test: a lookup
    /// scans about two thousand boxes and runs point-in-polygon only on the
    /// few polygons whose box holds the point.
    pub fn hit(&self, lat: f64, lon: f64) -> Option<usize> {
        if !lat.is_finite() || !lon.is_finite() {
            return None;
        }
        let lat = lat.clamp(-89.998, 89.998);
        let lon = wrap_lon(lon).min(180.0 - 1e-9);
        self.index
            .iter()
            .filter(|(b, _, _)| b[0] <= lon && lon <= b[2] && b[1] <= lat && lat <= b[3])
            .find(|(_, ci, pi)| polygon_contains(&self.list[*ci].polygons[*pi], lon, lat))
            .map(|(_, ci, _)| *ci)
    }

    /// Rasterize country IDs (`index + 1`, 0 for none) into a `width × width/2`
    /// equirectangular grid, row 0 at the north. Each texel takes the country
    /// that contains its center. Uses an even-odd scanline fill per polygon, so
    /// the cost follows the covered area, not the full grid.
    pub fn rasterize_ids(&self, width: usize) -> Vec<u8> {
        let height = width / 2;
        let mut out = vec![0u8; width * height];
        if height == 0 {
            return out;
        }
        let (dx, dy) = (360.0 / width as f64, 180.0 / height as f64);
        let mut xs: Vec<f64> = Vec::new();
        for (ci, country) in self.list.iter().enumerate() {
            let id = (ci + 1) as u8;
            for poly in &country.polygons {
                // Rows whose center latitude, 90 - (r + 0.5) dy, is in the box.
                let r0 = ((90.0 - poly.bbox[3]) / dy - 0.5).ceil().max(0.0) as usize;
                let r1 = ((90.0 - poly.bbox[1]) / dy - 0.5).floor();
                if r1 < 0.0 {
                    continue;
                }
                let r1 = (r1 as usize).min(height - 1);
                for row in r0..=r1 {
                    let lat = 90.0 - (row as f64 + 0.5) * dy;
                    xs.clear();
                    for ring in &poly.rings {
                        if lat < ring.bbox[1] || lat > ring.bbox[3] {
                            continue;
                        }
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
                    for pair in xs.windows(2).step_by(2) {
                        // Texel c has its center at lon = -180 + (c + 0.5) dx.
                        let c0 = ((pair[0] + 180.0) / dx - 0.5).ceil().max(0.0) as usize;
                        let c1 = ((pair[1] + 180.0) / dx - 0.5).ceil().max(0.0) as usize;
                        let c1 = c1.min(width);
                        if c0 < c1 {
                            out[row * width + c0..row * width + c1].fill(id);
                        }
                    }
                }
            }
        }
        out
    }
}

impl Country {
    /// Flag emoji built from the ISO alpha-2 code, if there is one.
    pub fn flag(&self) -> Option<String> {
        flag_emoji(&self.iso2)
    }

    /// Label/value rows for the info panel; empty values are omitted.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let code = match (self.iso2.is_empty(), self.iso3.is_empty()) {
            (false, false) => format!("{} / {}", self.iso2, self.iso3),
            (false, true) => self.iso2.clone(),
            (true, false) => self.iso3.clone(),
            (true, true) => String::new(),
        };
        let mut rows = vec![
            ("Continent", self.continent.clone()),
            ("Region", self.subregion.clone()),
            ("Population", format_population(self.population)),
            ("ISO code", code),
        ];
        rows.retain(|(_, v)| !v.is_empty());
        rows
    }
}

/// Regional-indicator flag for a two-letter uppercase ASCII code.
pub fn flag_emoji(iso2: &str) -> Option<String> {
    let b = iso2.as_bytes();
    if b.len() != 2 || !b.iter().all(u8::is_ascii_uppercase) {
        return None;
    }
    b.iter()
        .map(|&c| char::from_u32(0x1F1E6 + u32::from(c - b'A')))
        .collect()
}

/// `14645468` becomes `"14,645,468"`.
pub fn format_population(n: u64) -> String {
    let mut out = Vec::new();
    for (i, d) in n.to_string().bytes().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(b',');
        }
        out.push(d);
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Fetch and decode the country file.
pub async fn load() -> Result<Countries, JsValue> {
    let url = "data/ne_50m_admin_0_countries.bin";
    let bytes = crate::geo::fetch_bytes(url).await?;
    decode_countries(&bytes).map_err(|e| JsValue::from_str(&format!("{url}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    const BIN: &[u8] = include_bytes!("../assets/data/ne_50m_admin_0_countries.bin");
    const GEOJSON: &str = include_str!("../data-src/ne_50m_admin_0_countries.geojson");

    fn world() -> Countries {
        decode_countries(BIN).unwrap()
    }

    fn name_at(w: &Countries, lat: f64, lon: f64) -> Option<&str> {
        w.hit(lat, lon).map(|i| w.list[i].name.as_str())
    }

    #[test]
    fn known_places() {
        let w = world();
        for (lat, lon, want) in [
            (48.857, 2.352, "France"),
            (52.52, 13.405, "Germany"),
            (-33.87, 151.21, "Australia"),
            (35.68, 139.69, "Japan"),
            (-14.24, -51.93, "Brazil"),
            (64.1, -21.9, "Iceland"),
            (-1.29, 36.82, "Kenya"),
        ] {
            assert_eq!(name_at(&w, lat, lon), Some(want), "{lat},{lon}");
        }
    }

    #[test]
    fn ocean_is_no_country() {
        let w = world();
        for (lat, lon) in [(0.0, -30.0), (-45.0, -150.0), (89.0, 0.0), (0.0, 179.99)] {
            assert_eq!(name_at(&w, lat, lon), None, "{lat},{lon}");
        }
    }

    #[test]
    fn enclaves() {
        let w = world();
        assert_eq!(name_at(&w, -29.6, 28.2), Some("Lesotho"));
        assert_eq!(name_at(&w, -30.5, 25.0), Some("South Africa"));
    }

    #[test]
    fn russia_on_both_sides_of_the_seam() {
        let w = world();
        assert_eq!(name_at(&w, 55.75, 37.62), Some("Russia"));
        assert_eq!(name_at(&w, 66.0, 176.0), Some("Russia"), "west of the seam");
        assert_eq!(
            name_at(&w, 66.5, -172.5),
            Some("Russia"),
            "east of the seam"
        );
        assert_eq!(name_at(&w, 67.0, 180.0), Some("Russia"), "on the seam");
        assert_eq!(name_at(&w, 67.0, -180.0), Some("Russia"));
        assert_eq!(
            name_at(&w, 67.0, 540.0),
            Some("Russia"),
            "wrapped longitude"
        );
    }

    #[test]
    fn fiji_on_both_sides_of_the_seam() {
        let w = world();
        assert_eq!(name_at(&w, -17.8, 178.0), Some("Fiji"), "Viti Levu");
        assert_eq!(name_at(&w, -16.75, -179.9), Some("Fiji"), "Taveuni");
        assert_eq!(name_at(&w, -16.75, 180.1), Some("Fiji"), "lon > 180");
    }

    #[test]
    fn poles() {
        let w = world();
        assert_eq!(name_at(&w, -90.0, 0.0), Some("Antarctica"));
        assert_eq!(name_at(&w, -89.9, 123.0), Some("Antarctica"));
        assert_eq!(name_at(&w, -85.0, 179.9), Some("Antarctica"));
        assert_eq!(name_at(&w, -85.0, -179.9), Some("Antarctica"));
        assert_eq!(name_at(&w, 90.0, 0.0), None);
        assert_eq!(name_at(&w, 95.0, 0.0), None, "beyond the pole is clamped");
    }

    #[test]
    fn non_finite_input_is_a_miss() {
        let w = world();
        for (lat, lon) in [
            (f64::NAN, 0.0),
            (0.0, f64::NAN),
            (f64::INFINITY, 0.0),
            (0.0, f64::NEG_INFINITY),
        ] {
            assert_eq!(w.hit(lat, lon), None);
        }
    }

    #[test]
    fn wrap_lon_cases() {
        assert_eq!(wrap_lon(0.0), 0.0);
        assert_eq!(wrap_lon(180.0), 180.0);
        assert_eq!(wrap_lon(-180.0), -180.0);
        assert_eq!(wrap_lon(540.0), 180.0);
        assert_eq!(wrap_lon(190.0), -170.0);
        assert_eq!(wrap_lon(-190.0), 170.0);
        assert_eq!(wrap_lon(-540.0), -180.0);
    }

    #[test]
    fn attributes() {
        let w = world();
        let get = |n: &str| w.list.iter().find(|c| c.name == n).unwrap();
        let fr = get("France");
        assert_eq!((fr.iso2.as_str(), fr.iso3.as_str()), ("FR", "FRA"));
        assert_eq!(fr.continent, "Europe");
        assert_eq!(fr.subregion, "Western Europe");
        assert!(fr.population > 60_000_000);
        assert_eq!(fr.flag().as_deref(), Some("\u{1F1EB}\u{1F1F7}"));
        let somaliland = get("Somaliland");
        assert_eq!(somaliland.iso2, "", "-99 becomes empty");
        assert_eq!(somaliland.flag(), None);
        assert!(!somaliland.rows().is_empty());
        assert!(w.list.iter().all(|c| !c.name.is_empty()));
        assert!(w.len() > 200 && w.len() <= MAX_COUNTRIES);
    }

    #[test]
    fn formatting() {
        assert_eq!(format_population(0), "0");
        assert_eq!(format_population(999), "999");
        assert_eq!(format_population(1_000), "1,000");
        assert_eq!(format_population(14_645_468), "14,645,468");
        assert_eq!(format_population(1_400_000_000), "1,400,000,000");
        assert_eq!(flag_emoji("us"), None);
        assert_eq!(flag_emoji("USA"), None);
        assert_eq!(flag_emoji(""), None);
        assert_eq!(flag_emoji("US").as_deref(), Some("\u{1F1FA}\u{1F1F8}"));
    }

    // --- committed binary vs. GeoJSON ---

    #[derive(Deserialize)]
    struct Fc {
        features: Vec<Feature>,
    }
    #[derive(Deserialize)]
    struct Feature {
        properties: Props,
        geometry: Geometry,
    }
    #[derive(Deserialize)]
    struct Props {
        #[serde(rename = "NAME")]
        name: String,
        #[serde(rename = "POP_EST")]
        pop: f64,
    }
    #[derive(Deserialize)]
    #[serde(tag = "type", content = "coordinates")]
    enum Geometry {
        Polygon(Vec<Vec<Vec<f64>>>),
        MultiPolygon(Vec<Vec<Vec<Vec<f64>>>>),
    }

    #[test]
    fn binary_matches_geojson() {
        let fc: Fc = serde_json::from_str(GEOJSON).unwrap();
        let w = world();
        assert_eq!(fc.features.len(), w.len());
        for (f, c) in fc.features.iter().zip(&w.list) {
            assert_eq!(f.properties.name, c.name);
            assert_eq!(f.properties.pop as u64, c.population, "{}", c.name);
            let polys = match &f.geometry {
                Geometry::Polygon(p) => vec![p.clone()],
                Geometry::MultiPolygon(m) => m.clone(),
            };
            assert_eq!(polys.len(), c.polygons.len(), "{}: polygons", c.name);
            for (src, got) in polys.iter().zip(&c.polygons) {
                assert_eq!(src.len(), got.rings.len(), "{}: rings", c.name);
                for (sr, gr) in src.iter().zip(&got.rings) {
                    // The closing point is dropped.
                    assert_eq!(sr.len() - 1, gr.points.len(), "{}: points", c.name);
                    for (a, b) in sr.iter().zip(&gr.points) {
                        assert!((a[0] - b[0]).abs() <= 0.0005 + 1e-9, "{}", c.name);
                        assert!((a[1] - b[1]).abs() <= 0.0005 + 1e-9, "{}", c.name);
                    }
                }
            }
        }
    }

    #[test]
    fn bundled_data_is_in_bounds() {
        for c in &world().list {
            for p in &c.polygons {
                for r in &p.rings {
                    assert!(r.points.len() >= 3);
                    for &[lon, lat] in &r.points {
                        assert!((-180.0..=180.0).contains(&lon), "{}", c.name);
                        assert!((-90.0..=90.0).contains(&lat), "{}", c.name);
                    }
                }
            }
        }
    }

    // --- raster ---

    #[test]
    fn raster_agrees_with_hit_test() {
        let w = world();
        let width = 1024;
        let ids = w.rasterize_ids(width);
        assert_eq!(ids.len(), width * width / 2);
        let (mut agree, mut total) = (0, 0);
        for row in (0..width / 2).step_by(3) {
            for col in (0..width).step_by(3) {
                let lat = 90.0 - (row as f64 + 0.5) * 180.0 / (width / 2) as f64;
                let lon = -180.0 + (col as f64 + 0.5) * 360.0 / width as f64;
                let want = w.hit(lat, lon).map_or(0, |i| i as u8 + 1);
                total += 1;
                agree += (ids[row * width + col] == want) as usize;
            }
        }
        // Differences are limited to overlapping or touching polygons.
        assert!(agree as f64 / total as f64 > 0.999, "{agree}/{total}");
        // Spot checks on the raster itself, including both sides of the seam.
        let at = |lat: f64, lon: f64| {
            let row = ((90.0 - lat) / 180.0 * (width / 2) as f64) as usize;
            let col = ((lon + 180.0) / 360.0 * width as f64) as usize;
            ids[row * width + col]
        };
        let id_of = |n: &str| w.list.iter().position(|c| c.name == n).unwrap() as u8 + 1;
        assert_eq!(at(66.0, 176.0), id_of("Russia"));
        assert_eq!(at(66.5, -172.5), id_of("Russia"));
        assert_eq!(at(-17.8, 178.0), id_of("Fiji"));
        assert_eq!(at(-89.0, 0.0), id_of("Antarctica"));
        assert_eq!(at(0.0, -30.0), 0);
    }

    // --- synthetic data ---

    fn put(out: &mut Vec<u8>, mut n: u64) {
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    fn put_str(out: &mut Vec<u8>, s: &str) {
        put(out, s.len() as u64);
        out.extend_from_slice(s.as_bytes());
    }

    type Rings = Vec<Vec<[i64; 2]>>;

    /// One country per entry; polygons are lists of rings in whole degrees.
    fn build(countries: &[(&str, Vec<Rings>)]) -> Vec<u8> {
        let mut out = b"MAPC\x01".to_vec();
        put(&mut out, 1);
        put(&mut out, countries.len() as u64);
        let (mut px, mut py) = (0, 0);
        for (name, polys) in countries {
            for s in [name, "XX", "XXX", "Land", "Sub"] {
                put_str(&mut out, s);
            }
            put(&mut out, 1234);
            put(&mut out, polys.len() as u64);
            for rings in polys {
                put(&mut out, rings.len() as u64);
                for ring in rings {
                    put(&mut out, ring.len() as u64);
                    for &[x, y] in ring {
                        for d in [x - px, y - py] {
                            put(&mut out, ((d << 1) ^ (d >> 63)) as u64);
                        }
                        (px, py) = (x, y);
                    }
                }
            }
        }
        out
    }

    fn square(x0: i64, y0: i64, x1: i64, y1: i64) -> Vec<[i64; 2]> {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    #[test]
    fn synthetic_hole_and_multipolygon() {
        let data = build(&[
            (
                "Donut",
                vec![vec![square(0, 0, 40, 40), square(10, 10, 30, 30)]],
            ),
            ("Plug", vec![vec![square(15, 15, 25, 25)]]),
            (
                "Islands",
                vec![
                    vec![square(100, 0, 110, 10)],
                    vec![square(-110, 0, -100, 10)],
                ],
            ),
        ]);
        let w = decode_countries(&data).unwrap();
        assert_eq!(name_at(&w, 5.0, 5.0), Some("Donut"));
        assert_eq!(
            name_at(&w, 20.0, 20.0),
            Some("Plug"),
            "hole hosts a country"
        );
        assert_eq!(name_at(&w, 12.0, 12.0), None, "inside the hole");
        assert_eq!(name_at(&w, 41.0, 5.0), None);
        assert_eq!(name_at(&w, 5.0, 105.0), Some("Islands"));
        assert_eq!(name_at(&w, 5.0, -105.0), Some("Islands"));
        assert_eq!(name_at(&w, 5.0, -50.0), None);
        // The raster honors the hole too (one texel per degree).
        let ids = w.rasterize_ids(360);
        let at = |lat: f64, lon: f64| ids[((90.0 - lat) as usize) * 360 + (lon + 180.0) as usize];
        assert_eq!(at(5.5, 5.5), 1);
        assert_eq!(at(20.5, 20.5), 2);
        assert_eq!(at(12.5, 12.5), 0);
        assert_eq!(at(5.5, 105.5), 3);
    }

    #[test]
    fn polygons_touching_the_seam_rasterize_to_the_edge() {
        let data = build(&[
            ("East", vec![vec![square(170, -10, 180, 10)]]),
            ("West", vec![vec![square(-180, -10, -170, 10)]]),
        ]);
        let w = decode_countries(&data).unwrap();
        assert_eq!(name_at(&w, 0.0, 180.0), Some("East"));
        assert_eq!(name_at(&w, 0.0, -180.0), Some("West"));
        let ids = w.rasterize_ids(360);
        let row = 89 * 360;
        assert_eq!(ids[row + 359], 1);
        assert_eq!(ids[row], 2);
    }

    #[test]
    fn malformed_data_is_rejected() {
        let good = build(&[("A", vec![vec![square(0, 0, 10, 10)]])]);
        assert!(decode_countries(&good).is_ok());
        assert!(decode_countries(b"").is_err());
        assert!(decode_countries(b"MAPD\x01\x01\x00").is_err(), "bad magic");
        assert!(
            decode_countries(b"MAPC\x02\x01\x00").is_err(),
            "bad version"
        );
        assert!(decode_countries(b"MAPC\x01\x00\x00").is_err(), "zero scale");
        for len in 0..good.len() {
            assert!(
                decode_countries(&good[..len]).is_err(),
                "truncated at {len}"
            );
        }
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(decode_countries(&trailing).is_err());
        // Huge counts and string lengths must not allocate or panic.
        assert!(decode_countries(b"MAPC\x01\x01\xff\xff\xff\xff\x0f").is_err());
        assert!(decode_countries(b"MAPC\x01\x01\x01\xff\xff\xff\xff\x0f").is_err());
        // Coordinates beyond lon/lat range.
        let bad = build(&[("A", vec![vec![square(0, 0, 181, 10)]])]);
        assert!(decode_countries(&bad).is_err());
        let bad = build(&[("A", vec![vec![square(0, 0, 10, 91)]])]);
        assert!(decode_countries(&bad).is_err());
    }

    #[test]
    fn empty_file_has_no_countries() {
        let w = decode_countries(b"MAPC\x01\x01\x00").unwrap();
        assert_eq!(w.len(), 0);
        assert_eq!(w.hit(0.0, 0.0), None);
        assert!(w.rasterize_ids(8).iter().all(|&b| b == 0));
    }
}
