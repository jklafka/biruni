//! Loading GeoJSON and rasterizing it into an equirectangular map texture.
//!
//! The texture packs two layers into the red and green channels:
//! * R — land mask (lakes cut out), sampled with a 0.5 threshold for crisp
//!   coastlines at any zoom.
//! * G — country borders.
//!
//! Layers are fetched in a compact binary format (see [`decode_shapes`]),
//! produced from the Natural Earth GeoJSON in `data-src/` by
//! `scripts/convert_geojson.py`. The GeoJSON parser stays as the reference the
//! tests compare the binary files against.

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{CanvasRenderingContext2d, CanvasWindingRule, HtmlCanvasElement, Path2d, Response};

/// A polyline or polygon ring in lon/lat degrees.
#[derive(Debug, PartialEq)]
pub struct Shape {
    pub points: Vec<[f64; 2]>,
    /// Polygon rings are closed; line strings are not.
    pub closed: bool,
}

/// Reference GeoJSON parser, used by tests to check the binary files.
#[cfg(test)]
mod geojson {
    use super::Shape;
    use serde::Deserialize;

    type Position = Vec<f64>;

    #[derive(Deserialize)]
    struct FeatureCollection {
        features: Vec<Feature>,
    }

    #[derive(Deserialize)]
    struct Feature {
        geometry: Option<Geometry>,
    }

    #[derive(Deserialize)]
    #[serde(tag = "type", content = "coordinates")]
    enum Geometry {
        Point(serde::de::IgnoredAny),
        MultiPoint(serde::de::IgnoredAny),
        LineString(Vec<Position>),
        MultiLineString(Vec<Vec<Position>>),
        Polygon(Vec<Vec<Position>>),
        MultiPolygon(Vec<Vec<Vec<Position>>>),
    }

    /// Flatten a GeoJSON FeatureCollection into drawable shapes.
    ///
    /// Points are ignored, features without geometry are skipped, positions with
    /// fewer than two coordinates are dropped (extra ones, like altitude, are
    /// ignored), and shapes left with fewer than two points are discarded.
    pub fn parse_shapes(json: &str) -> Result<Vec<Shape>, serde_json::Error> {
        let fc: FeatureCollection = serde_json::from_str(json)?;
        let mut shapes = Vec::new();
        let mut push = |coords: &[Position], closed: bool| {
            let points: Vec<[f64; 2]> = coords
                .iter()
                .filter(|p| p.len() >= 2)
                .map(|p| [p[0], p[1]])
                .collect();
            if points.len() >= 2 {
                shapes.push(Shape { points, closed });
            }
        };
        for geometry in fc.features.into_iter().filter_map(|f| f.geometry) {
            match geometry {
                Geometry::Point(_) | Geometry::MultiPoint(_) => {}
                Geometry::LineString(line) => push(&line, false),
                Geometry::MultiLineString(lines) => lines.iter().for_each(|l| push(l, false)),
                Geometry::Polygon(rings) => rings.iter().for_each(|r| push(r, true)),
                Geometry::MultiPolygon(polys) => polys.iter().flatten().for_each(|r| push(r, true)),
            }
        }
        Ok(shapes)
    }
}

/// Binary layer format, version 1 (all integers are LEB128 varints):
///
/// ```text
/// "MAPD" | version u8 = 1 | shape_count
/// shape_count × ( (point_count << 1 | closed) | point_count × (dlon, dlat) )
/// ```
///
/// `dlon`/`dlat` are zigzag-encoded deltas, in units of 1/[`COORD_SCALE`]
/// degree, from the previous point (the first point of the file is relative to
/// 0,0 and later shapes continue from the last point of the previous shape).
/// Quantizing to 1e-4 degree (about 11 m) moves a coordinate by at most
/// [`COORD_TOLERANCE`] degrees, far below one texel of the 8192-wide map
/// (0.044 degrees).
pub const COORD_SCALE: f64 = 10_000.0;
/// Maximum error, in degrees, that quantization adds to a coordinate.
#[cfg(test)]
pub const COORD_TOLERANCE: f64 = 0.5 / COORD_SCALE;

const MAGIC: &[u8; 4] = b"MAPD";
const VERSION: u8 = 1;

pub(crate) fn read_varint(data: &[u8], pos: &mut usize) -> Result<u64, String> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *data.get(*pos).ok_or("unexpected end of data")?;
        *pos += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            // The tenth byte may only carry the top bit of a u64.
            if shift == 63 && byte > 1 {
                break;
            }
            return Ok(value);
        }
    }
    Err("varint too long".into())
}

pub(crate) fn read_delta(data: &[u8], pos: &mut usize) -> Result<i64, String> {
    let z = read_varint(data, pos)?;
    Ok((z >> 1) as i64 ^ -((z & 1) as i64))
}

/// Decode the compact binary format into shapes. Rejects malformed input
/// (bad header, truncation, trailing bytes, coordinates outside lon/lat
/// bounds) instead of panicking or allocating unbounded memory.
pub fn decode_shapes(data: &[u8]) -> Result<Vec<Shape>, String> {
    if data.len() < 5 || &data[..4] != MAGIC {
        return Err("not a map data file".into());
    }
    if data[4] != VERSION {
        return Err(format!("unsupported map data version {}", data[4]));
    }
    let mut pos = 5;
    let count = read_varint(data, &mut pos)?;
    // Every shape needs at least a header byte and two points (4 bytes).
    if count > (data.len() / 5) as u64 {
        return Err("shape count exceeds data size".into());
    }
    let mut shapes = Vec::with_capacity(count as usize);
    let (mut x, mut y) = (0i64, 0i64);
    let (max_x, max_y) = (180 * COORD_SCALE as i64, 90 * COORD_SCALE as i64);
    for _ in 0..count {
        let header = read_varint(data, &mut pos)?;
        let n = header >> 1;
        if n < 2 || n > ((data.len() - pos) / 2) as u64 {
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
            points.push([x as f64 / COORD_SCALE, y as f64 / COORD_SCALE]);
        }
        shapes.push(Shape {
            points,
            closed: header & 1 == 1,
        });
    }
    if pos != data.len() {
        return Err("trailing bytes after last shape".into());
    }
    Ok(shapes)
}

/// Vector layers as canvas paths in lon/lat degree coordinates.
pub struct Layers {
    land: Path2d,
    lakes: Path2d,
    borders: Path2d,
}

pub(crate) async fn fetch_bytes(url: &str) -> Result<Vec<u8>, JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let resp: Response = JsFuture::from(window.fetch_with_str(url))
        .await?
        .dyn_into()?;
    if !resp.ok() {
        return Err(format!("{url}: HTTP {}", resp.status()).into());
    }
    let buf = JsFuture::from(resp.array_buffer()?).await?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

async fn fetch_path(url: &str) -> Result<Path2d, JsValue> {
    let bytes = fetch_bytes(url).await?;
    let shapes = decode_shapes(&bytes).map_err(|e| JsValue::from_str(&format!("{url}: {e}")))?;
    let path = Path2d::new()?;
    for shape in &shapes {
        let [x0, y0] = shape.points[0];
        path.move_to(x0, y0);
        for &[x, y] in &shape.points[1..] {
            path.line_to(x, y);
        }
        if shape.closed {
            path.close_path();
        }
    }
    Ok(path)
}

impl Layers {
    pub async fn load() -> Result<Layers, JsValue> {
        let land = fetch_path("data/ne_50m_land.bin");
        let lakes = fetch_path("data/ne_50m_lakes.bin");
        let borders = fetch_path("data/ne_50m_admin_0_boundary_lines_land.bin");
        Ok(Layers {
            land: land.await?,
            lakes: lakes.await?,
            borders: borders.await?,
        })
    }

    /// Draw one tile of a `map_w × map_h` equirectangular map whose left edge
    /// sits `x_offset` pixels from the antimeridian, into `canvas`.
    pub fn draw_tile(
        &self,
        canvas: &HtmlCanvasElement,
        map_w: f64,
        map_h: f64,
        x_offset: f64,
    ) -> Result<(), JsValue> {
        let ctx: CanvasRenderingContext2d = canvas
            .get_context("2d")?
            .ok_or("2d canvas unavailable")?
            .dyn_into()?;
        let (w, h) = (canvas.width() as f64, canvas.height() as f64);

        ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)?;
        ctx.set_global_composite_operation("source-over")?;
        ctx.set_fill_style_str("#000");
        ctx.fill_rect(0.0, 0.0, w, h);

        // lon/lat degrees → pixels: x = (lon + 180) · sx − offset, y = (90 − lat) · sy
        let sx = map_w / 360.0;
        let sy = map_h / 180.0;
        ctx.set_transform(sx, 0.0, 0.0, -sy, 180.0 * sx - x_offset, 90.0 * sy)?;

        ctx.set_fill_style_str("#f00");
        ctx.fill_with_path_2d_and_winding(&self.land, CanvasWindingRule::Evenodd);
        ctx.set_fill_style_str("#000");
        ctx.fill_with_path_2d_and_winding(&self.lakes, CanvasWindingRule::Nonzero);

        ctx.set_global_composite_operation("lighter")?;
        ctx.set_stroke_style_str("#0f0");
        ctx.set_line_width(1.25 / sx);
        ctx.set_line_join("round");
        ctx.set_line_cap("round");
        ctx.stroke_with_path(&self.borders);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::geojson::parse_shapes;
    use super::*;

    fn fc(geometries: &[&str]) -> String {
        let features: Vec<String> = geometries
            .iter()
            .map(|g| format!(r#"{{"type":"Feature","properties":{{}},"geometry":{g}}}"#))
            .collect();
        format!(
            r#"{{"type":"FeatureCollection","name":"t","crs":{{"type":"name","properties":{{"name":"x"}}}},"features":[{}]}}"#,
            features.join(",")
        )
    }

    #[test]
    fn polygon_rings_are_closed_shapes() {
        let json = fc(&[
            r#"{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,0]],[[2,2],[3,2],[3,3],[2,2]]]}"#,
        ]);
        let shapes = parse_shapes(&json).unwrap();
        assert_eq!(shapes.len(), 2, "exterior ring and hole");
        assert!(shapes.iter().all(|s| s.closed));
        assert_eq!(shapes[0].points[1], [10.0, 0.0]);
    }

    #[test]
    fn multipolygon_flattens_all_rings() {
        let json = fc(&[
            r#"{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]],[[[5,5],[6,5],[6,6],[5,5]]]]}"#,
        ]);
        assert_eq!(parse_shapes(&json).unwrap().len(), 2);
    }

    #[test]
    fn lines_are_open_shapes() {
        let json = fc(&[
            r#"{"type":"LineString","coordinates":[[0,0],[1,1]]}"#,
            r#"{"type":"MultiLineString","coordinates":[[[0,0],[1,1]],[[2,2],[3,3],[4,4]]]}"#,
        ]);
        let shapes = parse_shapes(&json).unwrap();
        assert_eq!(shapes.len(), 3);
        assert!(shapes.iter().all(|s| !s.closed));
        assert_eq!(shapes[2].points.len(), 3);
    }

    #[test]
    fn points_and_null_geometries_are_skipped() {
        let json = fc(&[
            r#"{"type":"Point","coordinates":[1,2]}"#,
            r#"{"type":"MultiPoint","coordinates":[[1,2],[3,4]]}"#,
            "null",
            r#"{"type":"LineString","coordinates":[[0,0],[1,1]]}"#,
        ]);
        assert_eq!(parse_shapes(&json).unwrap().len(), 1);
    }

    #[test]
    fn third_coordinate_is_ignored_and_short_positions_dropped() {
        let json = fc(&[r#"{"type":"LineString","coordinates":[[0,0,100],[1],[2,2,5]]}"#]);
        let shapes = parse_shapes(&json).unwrap();
        assert_eq!(shapes[0].points, vec![[0.0, 0.0], [2.0, 2.0]]);
    }

    #[test]
    fn degenerate_shapes_are_discarded() {
        let json = fc(&[
            r#"{"type":"LineString","coordinates":[[0,0]]}"#,
            r#"{"type":"Polygon","coordinates":[[]]}"#,
        ]);
        assert!(parse_shapes(&json).unwrap().is_empty());
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(parse_shapes("{").is_err());
        assert!(
            parse_shapes(r#"{"features":[{"geometry":{"type":"Bogus","coordinates":[]}}]}"#)
                .is_err()
        );
    }

    const DATASETS: [(&str, &str, &[u8], usize); 3] = [
        (
            "land",
            include_str!("../data-src/ne_50m_land.geojson"),
            include_bytes!("../assets/data/ne_50m_land.bin"),
            1000,
        ),
        (
            "lakes",
            include_str!("../data-src/ne_50m_lakes.geojson"),
            include_bytes!("../assets/data/ne_50m_lakes.bin"),
            300,
        ),
        (
            "borders",
            include_str!("../data-src/ne_50m_admin_0_boundary_lines_land.geojson"),
            include_bytes!("../assets/data/ne_50m_admin_0_boundary_lines_land.bin"),
            300,
        ),
    ];

    fn assert_in_bounds(name: &str, shapes: &[Shape]) {
        for s in shapes {
            for &[lon, lat] in &s.points {
                assert!(
                    (-180.0..=180.0).contains(&lon) && (-90.0..=90.0).contains(&lat),
                    "{name}: point out of range: {lon},{lat}"
                );
            }
        }
    }

    /// The bundled files must decode, contain real data and stay in bounds.
    #[test]
    fn bundled_datasets_parse() {
        for (name, json, bin, min_shapes) in DATASETS {
            for shapes in [
                parse_shapes(json).unwrap_or_else(|e| panic!("{name}: {e}")),
                decode_shapes(bin).unwrap_or_else(|e| panic!("{name}: {e}")),
            ] {
                assert!(
                    shapes.len() >= min_shapes,
                    "{name}: only {} shapes",
                    shapes.len()
                );
                assert_in_bounds(name, &shapes);
            }
        }
    }

    /// The committed .bin files match the GeoJSON within the stated tolerance.
    #[test]
    fn binary_matches_geojson() {
        for (name, json, bin, _) in DATASETS {
            let want = parse_shapes(json).unwrap();
            let got = decode_shapes(bin).unwrap();
            assert_eq!(want.len(), got.len(), "{name}: shape count");
            let mut worst: f64 = 0.0;
            for (w, g) in want.iter().zip(&got) {
                assert_eq!(w.closed, g.closed, "{name}: closed flag");
                assert_eq!(w.points.len(), g.points.len(), "{name}: point count");
                for (a, b) in w.points.iter().zip(&g.points) {
                    worst = worst.max((a[0] - b[0]).abs()).max((a[1] - b[1]).abs());
                }
            }
            assert!(
                worst <= COORD_TOLERANCE + 1e-9,
                "{name}: max error {worst} exceeds {COORD_TOLERANCE}"
            );
        }
    }

    // --- encoder used only to build test inputs ---

    fn put_varint(out: &mut Vec<u8>, mut n: u64) {
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

    fn encode(shapes: &[Shape]) -> Vec<u8> {
        let mut out = b"MAPD\x01".to_vec();
        put_varint(&mut out, shapes.len() as u64);
        let (mut px, mut py) = (0i64, 0i64);
        for s in shapes {
            put_varint(&mut out, (s.points.len() as u64) << 1 | s.closed as u64);
            for &[lon, lat] in &s.points {
                let (x, y) = (
                    (lon * COORD_SCALE).round() as i64,
                    (lat * COORD_SCALE).round() as i64,
                );
                for d in [x - px, y - py] {
                    put_varint(&mut out, ((d << 1) ^ (d >> 63)) as u64);
                }
                (px, py) = (x, y);
            }
        }
        out
    }

    #[test]
    fn binary_round_trips_seam_and_poles() {
        let shapes = vec![
            Shape {
                points: vec![[-180.0, -90.0], [180.0, 90.0], [-179.99994, 0.00004]],
                closed: false,
            },
            Shape {
                points: vec![[179.9999, 89.9999], [-180.0, 0.0], [0.0, 0.0]],
                closed: true,
            },
        ];
        let got = decode_shapes(&encode(&shapes)).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].points[0], [-180.0, -90.0]);
        assert_eq!(got[0].points[1], [180.0, 90.0]);
        assert!(!got[0].closed && got[1].closed);
        for (w, g) in shapes.iter().zip(&got) {
            for (a, b) in w.points.iter().zip(&g.points) {
                assert!((a[0] - b[0]).abs() <= COORD_TOLERANCE + 1e-9);
                assert!((a[1] - b[1]).abs() <= COORD_TOLERANCE + 1e-9);
            }
        }
    }

    #[test]
    fn empty_file_decodes_to_no_shapes() {
        assert!(decode_shapes(b"MAPD\x01\x00").unwrap().is_empty());
    }

    #[test]
    fn malformed_binary_is_rejected() {
        let good = encode(&[Shape {
            points: vec![[1.0, 2.0], [3.0, 4.0]],
            closed: false,
        }]);
        assert!(decode_shapes(&good).is_ok());
        assert!(decode_shapes(b"").is_err());
        assert!(decode_shapes(b"MAP").is_err());
        assert!(decode_shapes(b"XXXX\x01\x00").is_err(), "bad magic");
        assert!(decode_shapes(b"MAPD\x02\x00").is_err(), "bad version");
        assert!(decode_shapes(b"MAPD\x01").is_err(), "missing count");
        for len in 0..good.len() {
            assert!(decode_shapes(&good[..len]).is_err(), "truncated at {len}");
        }
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(decode_shapes(&trailing).is_err(), "trailing byte");
        // Huge shape / point counts must not allocate or panic.
        assert!(decode_shapes(b"MAPD\x01\xff\xff\xff\xff\x0f").is_err());
        assert!(decode_shapes(b"MAPD\x01\x01\xfe\xff\xff\xff\x0f\x00\x00").is_err());
        // Overlong varint.
        assert!(decode_shapes(b"MAPD\x01\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\x01").is_err());
        // Fewer than two points in a shape.
        assert!(decode_shapes(b"MAPD\x01\x01\x02\x00\x00").is_err());
    }

    #[test]
    fn out_of_range_coordinates_are_rejected() {
        for (lon, lat) in [(180.0001, 0.0), (0.0, 90.0001), (-181.0, 0.0), (0.0, -91.0)] {
            let bad = encode(&[Shape {
                points: vec![[0.0, 0.0], [lon, lat]],
                closed: false,
            }]);
            assert!(decode_shapes(&bad).is_err(), "{lon},{lat}");
        }
    }

    /// Run with `cargo test --release -- --ignored --nocapture` to compare the
    /// native cost of the two parsers.
    #[test]
    #[ignore]
    fn bench_parsers() {
        use std::time::Instant;
        for (name, json, bin, _) in DATASETS {
            let t = Instant::now();
            let a = parse_shapes(json).unwrap();
            let json_t = t.elapsed();
            let t = Instant::now();
            let b = decode_shapes(bin).unwrap();
            let bin_t = t.elapsed();
            println!(
                "{name}: geojson {} B {:?} | bin {} B {:?} | {} shapes",
                json.len(),
                json_t,
                bin.len(),
                bin_t,
                a.len().max(b.len())
            );
        }
    }
}
