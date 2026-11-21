//! The deep time view's part of the app (`A`, `&ma=<age>`): the ages panel,
//! loading the plate model, and swapping the Earth view's land mask,
//! coastlines, country IDs, borders and names for the coastlines
//! reconstructed at an age (and back). The reconstruction is `paleo.rs`
//! (browser-free). It and the historical borders are never on together.

use crate::history::Timeline;
use crate::paleo::{self, Rotations};
use crate::view::Layer;
use crate::{data, element, listen, App};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsValue;
use web_sys::{Document, HtmlElement, HtmlInputElement, MouseEvent};

/// The age the view opens at: Pangaea.
pub const DEFAULT_MA: f64 = 250.0;

pub struct AgesUi {
    panel: HtmlElement,
    range: HtmlInputElement,
    value: HtmlElement,
    button: HtmlElement,
}

impl AgesUi {
    pub fn new(doc: &Document) -> Result<AgesUi, JsValue> {
        Ok(AgesUi {
            panel: element(doc, "ages")?,
            range: element(doc, "age-range")?,
            value: element(doc, "age-val")?,
            button: element(doc, "paleo-toggle")?,
        })
    }

    fn show(&self, on: bool, ma: f64) {
        self.panel.set_hidden(!on);
        let _ = self
            .button
            .set_attribute("aria-pressed", if on { "true" } else { "false" });
        // Time runs left to right: the slider counts up to the present.
        self.range.set_value(&format!("{}", paleo::MAX_MA - ma));
        let text = if ma > 0.0 {
            format!("{ma:.0} Ma · {}", paleo::period(ma))
        } else {
            "Present".into()
        };
        self.value.set_text_content(Some(&text));
        let _ = self
            .range
            .set_attribute("aria-valuetext", &paleo::format_ma(ma));
    }
}

#[derive(Default)]
pub struct PaleoState {
    pub on: bool,
    pub ma: f64,
    coasts: Option<Rc<Timeline>>,
    rotations: Option<Rc<Rotations>>,
    requested: bool,
    /// The age last drawn, and whether a redraw of the age is due.
    applied: Option<f64>,
    dirty: bool,
    prev_layer: Option<Layer>,
}

impl PaleoState {
    pub fn new(ma: Option<f64>) -> PaleoState {
        PaleoState {
            on: ma.is_some(),
            ma: ma.unwrap_or(DEFAULT_MA),
            ..PaleoState::default()
        }
    }
}

impl App {
    pub(crate) fn set_paleo(&mut self, on: bool) {
        let was = std::mem::replace(&mut self.paleo.on, on);
        if let Some(body) = self.window.document().and_then(|d| d.body()) {
            let _ = body.class_list().toggle_with_force("paleo-on", on);
        }
        self.ages_ui.show(on, self.paleo.ma);
        if on && !was {
            if self.history.on {
                self.set_history(false);
            }
            // Today's places mean nothing on old continents.
            if self.langs.on {
                self.set_languages(false);
            }
            if self.societies.on {
                self.set_societies(false);
            }
            if self.sun_on {
                self.toggle_sun();
            }
            self.select(None);
            let layer = self.layers_state.wanted();
            self.paleo.prev_layer = Some(layer);
            if layer != Layer::Political {
                self.select_layer(Layer::Political);
            }
            self.load_paleo();
            self.paleo_reapply();
        }
        if !on && was {
            if let Some(layers) = self.layers.clone() {
                // The modern land, ice, coastlines and borders.
                if let Err(e) = self.renderer.upload_map(&layers) {
                    web_sys::console::error_1(&e);
                }
            }
            self.restore_modern();
            self.paleo.applied = None;
            if let Some(l) = self.paleo.prev_layer.take() {
                self.select_layer(l);
            }
        }
        self.credits_for_layer();
        self.update_hover();
        self.redraw.mark_render();
        self.redraw.mark_hud();
        self.hash_stale = true;
    }

    pub(crate) fn set_ma(&mut self, ma: f64) {
        self.paleo.ma = ma.round().clamp(0.0, paleo::MAX_MA);
        self.ages_ui.show(self.paleo.on, self.paleo.ma);
        self.paleo.dirty = true;
        self.redraw.mark_render();
        self.hash_stale = true;
    }

    /// Draw the age again at the next frame (the modern map came back).
    pub(crate) fn paleo_reapply(&mut self) {
        let _ = self.canvas.remove_attribute("data-ma");
        self.paleo.applied = None;
        self.paleo.dirty = true;
        self.redraw.mark_render();
    }

    fn load_paleo(&mut self) {
        if std::mem::replace(&mut self.paleo.requested, true) {
            return;
        }
        let Some(app) = self.me.upgrade() else { return };
        wasm_bindgen_futures::spawn_local(async move {
            let coasts = match data::fetch("paleo_coastlines.bin").await {
                Ok(b) => crate::history::decode(&b).map_err(JsValue::from),
                Err(e) => Err(e),
            };
            let rot = match data::fetch("paleo_rotations.tsv").await {
                Ok(b) => String::from_utf8(b)
                    .map_err(|e| JsValue::from_str(&e.to_string()))
                    .and_then(|t| paleo::parse_rotations(&t).map_err(JsValue::from)),
                Err(e) => Err(e),
            };
            let mut a = app.borrow_mut();
            match (coasts, rot) {
                (Ok(c), Ok(r)) => {
                    a.paleo.coasts = Some(Rc::new(c));
                    a.paleo.rotations = Some(Rc::new(r));
                    a.paleo_reapply();
                }
                (Err(e), _) | (_, Err(e)) => {
                    a.paleo.requested = false;
                    web_sys::console::error_1(&e);
                }
            }
        });
    }

    /// Called every Earth frame: draw the age if it changed.
    pub(crate) fn paleo_frame(&mut self) {
        if !(self.paleo.on && self.paleo.dirty) || self.layers.is_none() {
            return;
        }
        let (Some(coasts), Some(rot)) = (self.paleo.coasts.clone(), self.paleo.rotations.clone())
        else {
            return;
        };
        self.paleo.dirty = false;
        if self.paleo.applied == Some(self.paleo.ma) {
            return;
        }
        let pieces = paleo::reconstruct(&coasts, &rot, self.paleo.ma);
        let width = self.renderer.ids_width();
        let groups = paleo::rasterize(&pieces, width);
        let land: Vec<u8> = groups
            .iter()
            .map(|&g| if g > 0 { 255 } else { 0 })
            .collect();
        let report = |r: Result<(), JsValue>| {
            if let Err(e) = r {
                web_sys::console::error_1(&e);
            }
        };
        report(self.renderer.upload_land_mask(&land, width));
        report(self.renderer.upload_country_ids(&groups, width));
        // A palette color per continent group, chosen so that neighbors
        // on the supercontinents differ.
        let mut colors = [0u8; 256];
        colors[..10].copy_from_slice(&paleo::GROUP_COLORS);
        self.renderer.set_country_colors(&colors);
        // No lines: the model's polygons tile the continents, so their
        // outlines would draw seams across the land. The coast is where the
        // land mask meets the sea.
        let none = crate::lines::build_levels(&[]);
        report(self.renderer.set_coast_lines(&none));
        report(self.renderer.set_border_lines(&none));
        self.labels.set_labels(Vec::new());
        self.paleo.applied = Some(self.paleo.ma);
        self.redraw.mark_render();
        self.redraw.mark_hud();
        self.set_globe_attr("data-ma", &format!("{}", self.paleo.ma));
    }

    /// Keys of the deep time view; returns whether the key was used.
    pub(crate) fn paleo_key(&mut self, key: &str, shift: bool) -> bool {
        let step = if shift || key == "<" || key == ">" {
            10.0
        } else {
            1.0
        };
        match key {
            "," | "<" => self.set_ma(self.paleo.ma + step),
            "." | ">" => self.set_ma(self.paleo.ma - step),
            _ => return false,
        }
        true
    }
}

/// The ages panel's slider and buttons.
pub fn wire(app: &Rc<RefCell<App>>, doc: &Document) -> Result<(), JsValue> {
    let range: HtmlInputElement = element(doc, "age-range")?;
    {
        let app = app.clone();
        let r = range.clone();
        listen(range.as_ref(), "input", true, move |_: web_sys::Event| {
            if let Ok(v) = r.value().parse::<f64>() {
                app.borrow_mut().set_ma(paleo::MAX_MA - v);
            }
        })?;
    }
    for (id, step) in [
        ("age-back", 10.0),
        ("age-prev", 1.0),
        ("age-next", -1.0),
        ("age-fwd", -10.0),
    ] {
        let button: HtmlElement = element(doc, id)?;
        let app = app.clone();
        listen(button.as_ref(), "click", true, move |_: MouseEvent| {
            let mut a = app.borrow_mut();
            let ma = a.paleo.ma + step;
            a.set_ma(ma);
        })?;
    }
    Ok(())
}
