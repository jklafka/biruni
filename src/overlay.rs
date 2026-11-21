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

/// What the sky view's overlay shows (see [`Overlay::draw_sky`]).
#[derive(Clone, Copy)]
pub struct SkyLabels<'a> {
    pub horizon: &'a [(f64, f64, &'a str)],
    pub stars: &'a [(f64, f64, &'a str)],
    /// Constellation lines: (thin, screen polyline).
    pub lines: &'a [(bool, Vec<(f64, f64)>)],
    pub figures: &'a [(f64, f64, &'a str)],
    /// Sun, Moon and planets, and how much daylight there is (the Moon's
    /// dark side is only dark at night).
    pub bodies: &'a [crate::sky_bodies::OnScreen],
    /// Where each body's name starts (`sky_bodies::label_spots`).
    pub body_labels: &'a [Option<(f64, f64)>],
    pub daylight: f64,
}

/// What the solar system view's overlay shows (see [`Overlay::draw_solar`]).
#[derive(Clone, Copy)]
pub struct SolarLabels<'a> {
    pub orbits: &'a [crate::solar_scene::OrbitLine],
    pub labels: &'a [crate::solar_scene::Label],
    pub target: crate::solar::Body,
    /// Spacecraft and small bodies, when shown and loaded.
    pub catalog: Option<&'a crate::space::Catalog>,
    pub object_paths: &'a [(usize, crate::solar_scene::Polylines)],
    pub markers: &'a [crate::solar_scene::objects::Marker],
    pub marker_labels: &'a [(usize, f64, f64)],
}

/// The languages layer's dots and names (see `languages.rs`).
pub struct LanguageMarks<'a> {
    pub langs: &'a crate::languages::Languages,
    pub dots: &'a [crate::languages::Dot],
    /// (language index, x, y): where each shown name starts.
    pub names: &'a [(usize, f64, f64)],
}

/// Dots with a dark rim, so every family color reads on any layer, then the
/// names.
fn draw_languages(ctx: &CanvasRenderingContext2d, l: &LanguageMarks) {
    use crate::languages::DOT_PX;
    ctx.set_line_width(1.0);
    ctx.set_stroke_style_str("rgba(0,0,0,0.6)");
    for d in l.dots {
        ctx.begin_path();
        let _ = ctx.arc(d.x, d.y, DOT_PX, 0.0, std::f64::consts::TAU);
        ctx.set_fill_style_str(l.langs.color(d.index));
        ctx.fill();
        ctx.stroke();
    }
    ctx.set_font("11px system-ui, -apple-system, Segoe UI, Roboto, sans-serif");
    ctx.set_text_align("left");
    ctx.set_text_baseline("middle");
    ctx.set_line_width(3.0);
    ctx.set_stroke_style_str("rgba(0,0,0,0.7)");
    ctx.set_fill_style_str("#f2f4f8");
    for (i, x, y) in l.names {
        let name = &l.langs.list[*i].name;
        let _ = ctx.stroke_text(name, *x, *y);
        let _ = ctx.fill_text(name, *x, *y);
    }
}

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

    /// Erase the overlay (during a projection morph).
    pub fn clear(&mut self) {
        if self.has_content {
            let _ = self.ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
            self.ctx.clear_rect(
                0.0,
                0.0,
                self.canvas.width() as f64,
                self.canvas.height() as f64,
            );
        }
    }

    /// Draw the sky view's overlay, replacing whatever was there:
    /// constellation lines and names, compass points centered just above
    /// their place on the horizon, and star names to the right of their star
    /// (CSS pixels).
    pub fn draw_sky(&mut self, l: &SkyLabels, css: (f64, f64), px: u32, py: u32, dpr: f64) {
        let SkyLabels {
            horizon,
            stars,
            lines,
            figures,
            bodies,
            body_labels,
            daylight,
        } = *l;
        if (px, py) != (self.canvas.width(), self.canvas.height()) {
            self.canvas.set_width(px);
            self.canvas.set_height(py);
        }
        let ctx = &self.ctx;
        let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
        ctx.clear_rect(0.0, 0.0, css.0, css.1);
        self.has_content = !(horizon.is_empty()
            && stars.is_empty()
            && lines.is_empty()
            && figures.is_empty()
            && bodies.is_empty());
        ctx.set_line_cap("round");
        ctx.set_line_width(1.0);
        for thin in [true, false] {
            ctx.set_stroke_style_str(if thin {
                "rgba(110,140,200,0.28)"
            } else {
                "rgba(120,155,220,0.55)"
            });
            ctx.begin_path();
            for (_, run) in lines.iter().filter(|(t, _)| *t == thin) {
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
        ctx.set_font("500 11px system-ui, -apple-system, Segoe UI, Roboto, sans-serif");
        ctx.set_text_align("center");
        ctx.set_text_baseline("middle");
        ctx.set_fill_style_str("rgba(140,170,225,0.8)");
        for (x, y, text) in figures {
            let _ = ctx.fill_text(&text.to_uppercase(), *x, *y);
        }
        ctx.set_line_width(3.0);
        ctx.set_stroke_style_str("rgba(0,0,0,0.7)");
        ctx.set_font("600 13px system-ui, -apple-system, Segoe UI, Roboto, sans-serif");
        ctx.set_text_align("center");
        ctx.set_text_baseline("bottom");
        ctx.set_fill_style_str("#c9d3e6");
        for (x, y, text) in horizon {
            let _ = ctx.stroke_text(text, *x, *y - 4.0);
            let _ = ctx.fill_text(text, *x, *y - 4.0);
        }
        ctx.set_font("12px system-ui, -apple-system, Segoe UI, Roboto, sans-serif");
        ctx.set_text_align("left");
        ctx.set_text_baseline("middle");
        ctx.set_fill_style_str("#9fb0cc");
        for (x, y, text) in stars {
            let _ = ctx.stroke_text(text, *x + 7.0, *y);
            let _ = ctx.fill_text(text, *x + 7.0, *y);
        }
        for b in bodies {
            draw_body(ctx, b, daylight);
        }
        ctx.set_font("600 12px system-ui, -apple-system, Segoe UI, Roboto, sans-serif");
        ctx.set_line_width(3.0);
        for (b, spot) in bodies.iter().zip(body_labels) {
            if let Some((x, y)) = spot {
                ctx.set_fill_style_str(b.color);
                let _ = ctx.stroke_text(b.name, *x, *y);
                let _ = ctx.fill_text(b.name, *x, *y);
            }
        }
    }

    /// Draw the solar system view's overlay, replacing whatever was there:
    /// the orbits (the target's a little stronger), the paths and markers of
    /// spacecraft and small bodies, and the names (CSS pixels).
    pub fn draw_solar(&mut self, l: &SolarLabels, css: (f64, f64), px: u32, py: u32, dpr: f64) {
        use crate::solar_scene::{css_color, objects};
        if (px, py) != (self.canvas.width(), self.canvas.height()) {
            self.canvas.set_width(px);
            self.canvas.set_height(py);
        }
        let ctx = &self.ctx;
        let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
        ctx.clear_rect(0.0, 0.0, css.0, css.1);
        self.has_content = true;
        ctx.set_line_cap("round");
        ctx.set_line_join("round");
        ctx.set_line_width(1.0);
        let stroke = |lines: &[Vec<(f64, f64)>]| {
            ctx.begin_path();
            for run in lines {
                for (i, (x, y)) in run.iter().enumerate() {
                    if i == 0 {
                        ctx.move_to(*x, *y);
                    } else {
                        ctx.line_to(*x, *y);
                    }
                }
            }
            ctx.stroke();
        };
        if let Some(cat) = l.catalog {
            for (i, lines) in l.object_paths {
                let kind = cat.objects[*i].kind;
                ctx.set_global_alpha(match kind {
                    crate::space::Kind::Spacecraft => 0.32,
                    crate::space::Kind::Comet => 0.18,
                    crate::space::Kind::Asteroid => 0.12,
                });
                ctx.set_stroke_style_str(objects::css_color(kind));
                stroke(lines);
            }
        }
        for o in l.orbits {
            ctx.set_global_alpha(if o.body == l.target { 0.7 } else { 0.38 });
            ctx.set_stroke_style_str(&css_color(o.body));
            stroke(&o.lines);
        }
        ctx.set_global_alpha(1.0);
        if let Some(cat) = l.catalog {
            for m in l.markers {
                let color = objects::css_color(m.kind);
                ctx.set_fill_style_str(color);
                ctx.begin_path();
                match m.kind {
                    crate::space::Kind::Spacecraft => {
                        let r = 3.2;
                        ctx.move_to(m.x, m.y - r);
                        ctx.line_to(m.x + r, m.y);
                        ctx.line_to(m.x, m.y + r);
                        ctx.line_to(m.x - r, m.y);
                        ctx.close_path();
                    }
                    _ => {
                        let _ = ctx.arc(m.x, m.y, 1.8, 0.0, std::f64::consts::TAU);
                    }
                }
                ctx.fill();
                if let Some((tx, ty)) = m.tail {
                    ctx.set_stroke_style_str(color);
                    ctx.set_line_width(1.5);
                    ctx.set_global_alpha(0.6);
                    ctx.begin_path();
                    ctx.move_to(m.x + tx * 2.0, m.y + ty * 2.0);
                    ctx.line_to(m.x + tx * 12.0, m.y + ty * 12.0);
                    ctx.stroke();
                    ctx.set_global_alpha(1.0);
                    ctx.set_line_width(1.0);
                }
            }
            ctx.set_font("11px system-ui, -apple-system, Segoe UI, Roboto, sans-serif");
            ctx.set_text_align("left");
            ctx.set_text_baseline("middle");
            ctx.set_line_width(3.0);
            ctx.set_stroke_style_str("rgba(0,0,0,0.7)");
            for (i, x, y) in l.marker_labels {
                let o = &cat.objects[*i];
                ctx.set_fill_style_str(objects::css_color(o.kind));
                let _ = ctx.stroke_text(&o.name, *x, *y);
                let _ = ctx.fill_text(&o.name, *x, *y);
            }
        }
        ctx.set_font("600 12px system-ui, -apple-system, Segoe UI, Roboto, sans-serif");
        ctx.set_text_align("left");
        ctx.set_text_baseline("middle");
        ctx.set_line_width(3.0);
        ctx.set_stroke_style_str("rgba(0,0,0,0.7)");
        for lb in l.labels {
            let name = lb.body.name();
            ctx.set_fill_style_str(&css_color(lb.body));
            let _ = ctx.stroke_text(name, lb.x, lb.y);
            let _ = ctx.fill_text(name, lb.x, lb.y);
        }
    }

    /// Redraw for the current camera. Does nothing when there is nothing to
    /// draw now and nothing to clear.
    pub fn draw(
        &mut self,
        cam: &Camera,
        measure: &Measure,
        langs: Option<&LanguageMarks>,
        px: u32,
        py: u32,
        dpr: f64,
    ) {
        if (px, py) != (self.canvas.width(), self.canvas.height()) {
            self.canvas.set_width(px);
            self.canvas.set_height(py);
            self.has_content = true; // resizing cleared it; redraw below
        }
        let measuring = measure.active || !measure.points().is_empty();
        let show = measuring || langs.is_some_and(|l| !l.dots.is_empty());
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
        if let Some(l) = langs {
            draw_languages(ctx, l);
        }
        if !measuring {
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
            let selected = measure.selected() == Some(i);
            let hover = measure.hover() == Some(i);
            if selected || hover {
                // A halo: solid blue when selected, faint white on hover.
                ctx.begin_path();
                let _ = ctx.arc(x, y, 11.0, 0.0, std::f64::consts::TAU);
                ctx.set_line_width(3.0);
                ctx.set_stroke_style_str(if selected {
                    "#3aa0ff"
                } else {
                    "rgba(255,255,255,0.75)"
                });
                ctx.stroke();
            }
            ctx.begin_path();
            let r = if selected || hover { 7.0 } else { 5.0 };
            let _ = ctx.arc(x, y, r, 0.0, std::f64::consts::TAU);
            ctx.set_fill_style_str(if selected || i == 0 {
                "#ffffff"
            } else {
                "#ffd24a"
            });
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
            _ => "Drag a point to move it, double-click to remove. Click beside a selected point, or Shift+click, to add one",
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

/// The Sun as a glowing disk, the Moon with its phase (lit side towards
/// `sun_angle`), a planet as a colored dot.
fn draw_body(ctx: &CanvasRenderingContext2d, b: &crate::sky_bodies::OnScreen, daylight: f64) {
    use crate::sky_bodies::Kind;
    use std::f64::consts::{FRAC_PI_2, PI, TAU};
    match b.kind {
        Kind::Sun => {
            if let Ok(g) = ctx.create_radial_gradient(b.x, b.y, b.r * 0.8, b.x, b.y, b.r * 4.0) {
                let _ = g.add_color_stop(0.0, "rgba(255,240,200,0.55)");
                let _ = g.add_color_stop(1.0, "rgba(255,240,200,0)");
                ctx.set_fill_style_canvas_gradient(&g);
                ctx.begin_path();
                let _ = ctx.arc(b.x, b.y, b.r * 4.0, 0.0, TAU);
                ctx.fill();
            }
            ctx.set_fill_style_str(b.color);
            ctx.begin_path();
            let _ = ctx.arc(b.x, b.y, b.r, 0.0, TAU);
            ctx.fill();
        }
        Kind::Moon => {
            // The dark side hides the stars behind it at night; by day it
            // is the sky's color, so leave it out.
            let dark = 1.0 - daylight;
            if dark > 0.0 {
                ctx.set_fill_style_str(&format!("rgba(26,29,36,{dark:.3})"));
                ctx.begin_path();
                let _ = ctx.arc(b.x, b.y, b.r, 0.0, TAU);
                ctx.fill();
            }
            // Lit part: the half facing the Sun, closed by the terminator,
            // an ellipse whose half-width is r times |2 lit - 1|.
            let k = 2.0 * b.lit - 1.0;
            ctx.save();
            let _ = ctx.translate(b.x, b.y);
            let _ = ctx.rotate(b.sun_angle);
            ctx.set_fill_style_str(b.color);
            ctx.begin_path();
            let _ = ctx.arc(0.0, 0.0, b.r, -FRAC_PI_2, FRAC_PI_2);
            let rx = (b.r * k.abs()).max(0.01);
            // Gibbous: the terminator bulges into the dark half; crescent:
            // it stays on the lit half.
            let _ = ctx.ellipse_with_anticlockwise(
                0.0,
                0.0,
                rx,
                b.r,
                0.0,
                FRAC_PI_2,
                PI + FRAC_PI_2,
                k < 0.0,
            );
            ctx.fill();
            ctx.restore();
        }
        Kind::Planet => {
            ctx.set_fill_style_str(b.color);
            ctx.begin_path();
            let _ = ctx.arc(b.x, b.y, b.r, 0.0, TAU);
            ctx.fill();
        }
    }
}
