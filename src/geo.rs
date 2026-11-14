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
    let fc: FeatureCollection =
        serde_json::from_str(&text).map_err(|e| JsValue::from_str(&format!("{url}: {e}")))?;
    let path = Path2d::new()?;
    for geometry in fc.features.into_iter().filter_map(|f| f.geometry) {
        match geometry {
            Geometry::Point(_) | Geometry::MultiPoint(_) => {}
            Geometry::LineString(line) => add_line(&path, &line, false),
            Geometry::MultiLineString(lines) => {
                lines.iter().for_each(|l| add_line(&path, l, false))
            }
            Geometry::Polygon(rings) => rings.iter().for_each(|r| add_line(&path, r, true)),
            Geometry::MultiPolygon(polys) => polys
                .iter()
                .flatten()
                .for_each(|r| add_line(&path, r, true)),
        }
    }
    Ok(path)
}

fn add_line(path: &Path2d, coords: &[Position], close: bool) {
    let mut points = coords.iter().filter(|p| p.len() >= 2);
    if let Some(first) = points.next() {
        path.move_to(first[0], first[1]);
        for p in points {
            path.line_to(p[0], p[1]);
        }
        if close {
            path.close_path();
        }
    }
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
