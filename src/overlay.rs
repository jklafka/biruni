//! 2D canvas overlay drawn above the globe: the measurement path and the
//! measure-mode crosshair.
//!
//! Why an overlay rather than the shader: the path is a few hundred sampled
//! great-circle points projected with `Camera::project`, the same math that
//! picking uses, so it lines up with the ground exactly. Canvas strokes have
//! a constant width in screen pixels for free, and the globe shader and its
//! uniforms stay untouched.

use crate::camera::Camera;
use crate::measure::{format_distance, screen_path, Measure};
use crate::view::Units;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, Document, HtmlCanvasElement, HtmlElement};

pub struct Overlay {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    /// Something is drawn, so it must be cleared when it goes away.
    has_content: bool,
}

impl Overlay {
    pub fn new(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let ctx = canvas
            .get_context("2d")?
            .ok_or("no 2d context for the overlay")?
            .dyn_into::<CanvasRenderingContext2d>()?;
        Ok(Self {
            canvas,
            ctx,
            has_content: false,
        })
    }

    /// Redraw for the current camera. Does nothing when there is nothing to
    /// draw now and nothing to clear.
    pub fn draw(&mut self, cam: &Camera, measure: &Measure, px: u32, py: u32, dpr: f64) {
        if (px, py) != (self.canvas.width(), self.canvas.height()) {
            self.canvas.set_width(px);
            self.canvas.set_height(py);
            self.has_content = true; // resizing cleared it; redraw below
        }
        let show = measure.active || !measure.points().is_empty();
        if !show && !self.has_content {
            return;
        }
        let ctx = &self.ctx;
        let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
        ctx.clear_rect(0.0, 0.0, cam.width, cam.height);
        self.has_content = show;
        if !show {
            return;
        }
        ctx.set_line_cap("round");
        ctx.set_line_join("round");

        let runs = screen_path(cam, measure.points());
        // Dark casing under a bright core keeps the line readable on any layer.
        for (width, color) in [(5.0, "rgba(0,0,0,0.65)"), (2.5, "#ffd24a")] {
            ctx.set_line_width(width);
            ctx.set_stroke_style_str(color);
            ctx.begin_path();
            for run in &runs {
                for (i, (x, y)) in run.iter().enumerate() {
                    if i == 0 {
                        ctx.move_to(*x, *y);
                    } else {
                        ctx.line_to(*x, *y);
                    }
                }
            }
            ctx.stroke();
        }

        for (i, p) in measure.points().iter().enumerate() {
            let Some((x, y)) = cam.project(p.0, p.1) else {
                continue;
            };
            ctx.begin_path();
            let _ = ctx.arc(x, y, 5.0, 0.0, std::f64::consts::TAU);
            ctx.set_fill_style_str(if i == 0 { "#ffffff" } else { "#ffd24a" });
            ctx.fill();
            ctx.set_line_width(2.0);
            ctx.set_stroke_style_str("rgba(0,0,0,0.8)");
            ctx.stroke();
        }

        // Crosshair at the view center, where Enter places a point.
        let (cx, cy) = (cam.width * 0.5, cam.height * 0.5);
        ctx.set_line_width(1.5);
        ctx.set_stroke_style_str("rgba(255,255,255,0.8)");
        ctx.begin_path();
        for (dx, dy) in [(1.0, 0.0), (0.0, 1.0)] {
            ctx.move_to(cx - 9.0 * dx, cy - 9.0 * dy);
            ctx.line_to(cx - 3.0 * dx, cy - 3.0 * dy);
            ctx.move_to(cx + 3.0 * dx, cy + 3.0 * dy);
            ctx.line_to(cx + 9.0 * dx, cy + 9.0 * dy);
        }
        ctx.stroke();
    }
}

/// The measurement readout: total, per-segment list and hint.
pub struct Panel {
    root: HtmlElement,
    total: HtmlElement,
    hint: HtmlElement,
    legs: HtmlElement,
    toggle: HtmlElement,
    last: String,
}

impl Panel {
    pub fn new(doc: &Document) -> Result<Self, JsValue> {
        let get = |id: &str| -> Result<HtmlElement, JsValue> {
            doc.get_element_by_id(id)
                .ok_or_else(|| JsValue::from_str(&format!("missing #{id}")))?
                .dyn_into::<HtmlElement>()
                .map_err(|_| JsValue::from_str(&format!("#{id} has the wrong element type")))
        };
        Ok(Self {
            root: get("measure")?,
            total: get("measure-total")?,
            hint: get("measure-hint")?,
            legs: get("measure-legs")?,
            toggle: get("measure-toggle")?,
            last: String::new(),
        })
    }

    /// Update the readout. Touches the DOM only when the content changed.
    pub fn update(&mut self, measure: &Measure, units: Units) {
        let legs = measure.legs_km();
        let total = format_distance(measure.total_km(), units);
        let n = measure.points().len();
        let sig = format!("{}|{n}|{total}|{legs:?}|{units:?}", measure.active);
        if sig == self.last {
            return;
        }
        self.last = sig;
        self.root.set_hidden(!measure.active);
        let _ = self.toggle.set_attribute(
            "aria-pressed",
            if measure.active { "true" } else { "false" },
        );
        self.total.set_text_content(Some(&total));
        let hint = match measure.points().len() {
            0 => "Click or tap the globe to start",
            1 => "Add a second point",
            _ => "",
        };
        self.hint.set_text_content(Some(hint));
        let _ = self
            .hint
            .style()
            .set_property("display", if hint.is_empty() { "none" } else { "" });
        self.legs.set_inner_html("");
        if let Some(doc) = self.legs.owner_document() {
            for (i, km) in legs.iter().enumerate() {
                if let Ok(li) = doc.create_element("li") {
                    li.set_text_content(Some(&format!(
                        "{}→{}  {}",
                        i + 1,
                        i + 2,
                        format_distance(*km, units)
                    )));
                    let _ = self.legs.append_child(&li);
                }
            }
        }
    }
}
