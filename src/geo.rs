//! Loading GeoJSON and rasterizing it into an equirectangular map texture.
//!
//! The texture packs two layers into the red and green channels:
//! * R — land mask (lakes cut out), sampled with a 0.5 threshold for crisp
//!   coastlines at any zoom.
//! * G — country borders.

use serde::Deserialize;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{CanvasRenderingContext2d, CanvasWindingRule, HtmlCanvasElement, Path2d, Response};

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

/// A polyline or polygon ring in lon/lat degrees.
#[derive(Debug, PartialEq)]
pub struct Shape {
    pub points: Vec<[f64; 2]>,
    /// Polygon rings are closed; line strings are not.
    pub closed: bool,
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

/// Vector layers as canvas paths in lon/lat degree coordinates.
pub struct Layers {
    land: Path2d,
    lakes: Path2d,
    borders: Path2d,
}

async fn fetch_text(url: &str) -> Result<String, JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let resp: Response = JsFuture::from(window.fetch_with_str(url))
        .await?
        .dyn_into()?;
    if !resp.ok() {
        return Err(format!("{url}: HTTP {}", resp.status()).into());
    }
    let text = JsFuture::from(resp.text()?).await?;
    text.as_string()
        .ok_or_else(|| "response was not text".into())
}

async fn fetch_path(url: &str) -> Result<Path2d, JsValue> {
    let text = fetch_text(url).await?;
    let shapes = parse_shapes(&text).map_err(|e| JsValue::from_str(&format!("{url}: {e}")))?;
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
        let land = fetch_path("data/ne_50m_land.geojson");
        let lakes = fetch_path("data/ne_50m_lakes.geojson");
        let borders = fetch_path("data/ne_50m_admin_0_boundary_lines_land.geojson");
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

    /// The bundled Natural Earth files must parse and contain real data.
    #[test]
    fn bundled_datasets_parse() {
        for (name, json, min_shapes) in [
            (
                "land",
                include_str!("../assets/data/ne_50m_land.geojson"),
                1000,
            ),
            (
                "lakes",
                include_str!("../assets/data/ne_50m_lakes.geojson"),
                300,
            ),
            (
                "borders",
                include_str!("../assets/data/ne_50m_admin_0_boundary_lines_land.geojson"),
                300,
            ),
        ] {
            let shapes = parse_shapes(json).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(
                shapes.len() >= min_shapes,
                "{name}: only {} shapes",
                shapes.len()
            );
            for s in &shapes {
                for &[lon, lat] in &s.points {
                    assert!(
                        (-180.0..=180.0).contains(&lon) && (-90.0..=90.0).contains(&lat),
                        "{name}: point out of range: {lon},{lat}"
                    );
                }
            }
        }
    }
}
