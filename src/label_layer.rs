//! Draws place labels on their own 2D canvas above the globe.
//!
//! All decisions (what shows, where, in what order) live in the browser-free
//! `labels.rs`. This file measures text once, keeps each label's fade state,
//! and paints. The canvas has `pointer-events: none`, so hover, click and drag
//! reach the globe through the labels.
//!
//! Work happens only on frames where the view changed or a fade is running.
//! An idle globe costs nothing.

use crate::camera::Camera;
use crate::labels::{self, Kind, Label, Rect};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    CanvasRenderingContext2d, Element, HtmlCanvasElement, MediaQueryList, ResizeObserver,
};

const FAMILY: &str = "ui-sans-serif, system-ui, -apple-system, \"Segoe UI\", sans-serif";

pub struct LabelLayer {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    labels: Vec<Label>,
    /// Current opacity of every label that is visible or fading.
    alpha: HashMap<usize, f64>,
    /// Labels selected on the last update (fading in or fully shown).
    target: HashSet<usize>,
    /// Screen rectangles covered by UI panels; no label is placed under them.
    blocked: Vec<Rect>,
    reduced_motion: Option<MediaQueryList>,
    animating: bool,
    has_content: bool,
    /// `data-ready` is set on the canvas.
    flagged_ready: bool,
}

fn font(kind: Kind) -> String {
    let px = kind.font_px();
    match kind {
        Kind::Country => format!("600 {px}px {FAMILY}"),
        Kind::City => format!("{px}px {FAMILY}"),
        Kind::Ocean => format!("italic {px}px {FAMILY}"),
    }
}

impl LabelLayer {
    pub fn new(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let ctx = canvas
            .get_context("2d")?
            .ok_or("no 2d context for the labels")?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let reduced_motion = web_sys::window()
            .and_then(|w| w.match_media("(prefers-reduced-motion: reduce)").ok())
            .flatten();
        Ok(Self {
            canvas,
            ctx,
            labels: Vec::new(),
            alpha: HashMap::new(),
            target: HashSet::new(),
            blocked: Vec::new(),
            reduced_motion,
            animating: false,
            has_content: false,
            flagged_ready: false,
        })
    }

    /// Install freshly loaded labels and measure their text.
    pub fn set_labels(&mut self, mut labels: Vec<Label>) {
        for kind in [Kind::Country, Kind::City, Kind::Ocean] {
            self.ctx.set_font(&font(kind));
            for l in labels.iter_mut().filter(|l| l.kind == kind) {
                l.width = self
                    .ctx
                    .measure_text(&l.text)
                    .map(|m| m.width())
                    .unwrap_or(l.text.chars().count() as f64 * 6.5);
            }
        }
        self.labels = labels;
        self.alpha.clear();
        self.target.clear();
    }

    /// Replace the panel rectangles. Returns true if they changed, in which
    /// case the next `update` re-selects labels.
    pub fn set_blocked(&mut self, blocked: Vec<Rect>) -> bool {
        if blocked == self.blocked {
            return false;
        }
        self.blocked = blocked;
        true
    }

    /// Remove every label at once (during a projection morph, when no fixed
    /// position is right). They fade back in afterwards.
    pub fn clear(&mut self) {
        if self.alpha.is_empty() && !self.has_content {
            return;
        }
        self.alpha.clear();
        self.target.clear();
        self.animating = false;
        self.has_content = false;
        let _ = self.ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        self.ctx.clear_rect(
            0.0,
            0.0,
            self.canvas.width() as f64,
            self.canvas.height() as f64,
        );
    }

    /// A fade is running, so the next frame must update again.
    pub fn animating(&self) -> bool {
        self.animating
    }

    /// Select, fade and paint for the current camera. Call on frames where
    /// the view changed or `animating()` is true.
    pub fn update(&mut self, cam: &Camera, dt_ms: f64, px: u32, py: u32, dpr: f64) {
        if (px, py) != (self.canvas.width(), self.canvas.height()) {
            self.canvas.set_width(px);
            self.canvas.set_height(py);
        }
        if self.labels.is_empty() && !self.has_content {
            return;
        }
        let reduced = self
            .reduced_motion
            .as_ref()
            .map(|m| m.matches())
            .unwrap_or(false);

        let picked = labels::select(cam, &self.labels, &self.blocked, |i| {
            self.target.contains(&i)
        });
        self.target = picked.iter().map(|p| p.index).collect();
        let facing: HashMap<usize, f64> = picked.iter().map(|p| (p.index, p.facing)).collect();

        // Selected labels fade toward 1, everything else toward 0.
        for p in &picked {
            self.alpha.entry(p.index).or_insert(0.0);
        }
        let target = &self.target;
        let mut animating = false;
        self.alpha.retain(|i, a| {
            let goal = if target.contains(i) { 1.0 } else { 0.0 };
            *a = labels::step_alpha(*a, goal, dt_ms, reduced);
            if *a != goal {
                animating = true;
            }
            *a > 0.0 || goal > 0.0
        });
        self.animating = animating;

        let ctx = &self.ctx;
        let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
        ctx.clear_rect(0.0, 0.0, cam.width, cam.height);
        ctx.set_line_join("round");
        ctx.set_text_baseline("middle");
        self.has_content = !self.alpha.is_empty();

        // Draw in kind order so each font is set once per frame.
        for kind in [Kind::Ocean, Kind::Country, Kind::City] {
            ctx.set_font(&font(kind));
            ctx.set_text_align(if kind == Kind::City { "left" } else { "center" });
            for (&i, &a) in &self.alpha {
                let l = &self.labels[i];
                if l.kind != kind {
                    continue;
                }
                let Some((x, y)) = cam.project(l.lat, l.lon) else {
                    continue;
                };
                // Labels fading out keep their own facing.
                let f = facing
                    .get(&i)
                    .copied()
                    .unwrap_or_else(|| labels::facing(cam, l.lat, l.lon));
                let alpha = a * f;
                if alpha <= 0.01 {
                    continue;
                }
                ctx.set_global_alpha(alpha);
                let tx = labels::text_x(kind, x);
                ctx.set_stroke_style_str("rgba(5,10,20,0.85)");
                ctx.set_line_width(3.0);
                let _ = ctx.stroke_text(&l.text, tx, y);
                ctx.set_fill_style_str(match kind {
                    Kind::Ocean => "#a9c9f5",
                    Kind::Country => "#ffffff",
                    Kind::City => "#f3f3f3",
                });
                let _ = ctx.fill_text(&l.text, tx, y);
                if kind == Kind::City {
                    ctx.begin_path();
                    let _ = ctx.arc(x, y, labels::DOT_R, 0.0, std::f64::consts::TAU);
                    ctx.set_stroke_style_str("rgba(5,10,20,0.85)");
                    ctx.set_line_width(1.5);
                    ctx.stroke();
                    ctx.set_fill_style_str("#ffffff");
                    ctx.fill();
                }
            }
        }
        ctx.set_global_alpha(1.0);

        // Tells the screenshot tests that labels are in place.
        if !self.flagged_ready && !animating && !self.alpha.is_empty() {
            self.flagged_ready = true;
            let _ = self.canvas.set_attribute("data-ready", "1");
        }
    }
}

/// Fetch and parse `data/labels.tsv`.
pub async fn load() -> Result<Vec<Label>, JsValue> {
    let bytes = crate::geo::fetch_bytes("data/labels.tsv").await?;
    let text =
        String::from_utf8(bytes).map_err(|_| JsValue::from_str("labels.tsv is not UTF-8"))?;
    labels::parse(&text).map_err(|e| JsValue::from_str(&format!("data/labels.tsv: {e}")))
}

/// Ids of the UI panels labels must stay out from under.
const PANELS: &[&str] = &[
    "title",
    "search",
    "layers",
    "hud",
    "controls",
    "country-info",
    "measure",
    "scrubber",
    "toast",
    "share-fallback",
];

/// Watches the UI panels and reports their screen rectangles. The DOM is
/// read only after a panel changes size or is shown or hidden (a
/// `ResizeObserver` fires for both), never every frame.
pub struct KeepOut {
    elements: Vec<Element>,
    dirty: Rc<Cell<bool>>,
    _observer: ResizeObserver,
}

impl KeepOut {
    pub fn new(doc: &web_sys::Document) -> Result<Self, JsValue> {
        let dirty = Rc::new(Cell::new(true));
        let flag = dirty.clone();
        let cb = Closure::<dyn FnMut()>::new(move || flag.set(true));
        let observer = ResizeObserver::new(cb.as_ref().unchecked_ref())?;
        cb.forget(); // lives for the whole page
        let mut elements = Vec::new();
        for id in PANELS {
            let el = doc
                .get_element_by_id(id)
                .ok_or_else(|| JsValue::from_str(&format!("missing #{id}")))?;
            observer.observe(&el);
            elements.push(el);
        }
        Ok(Self {
            elements,
            dirty,
            _observer: observer,
        })
    }

    /// Ask for a re-read on the next `take`, e.g. after the window resized.
    pub fn invalidate(&self) {
        self.dirty.set(true);
    }

    /// The rectangles of the visible panels if anything changed since the last call.
    pub fn take(&self, width: f64, height: f64) -> Option<Vec<Rect>> {
        if !self.dirty.replace(false) {
            return None;
        }
        Some(
            self.elements
                .iter()
                .filter_map(|el| {
                    let r = el.get_bounding_client_rect();
                    Rect::from_dom(r.x(), r.y(), r.width(), r.height(), width, height)
                })
                .collect(),
        )
    }
}
