//! The societies layer's part of the app (`K`, `&societies=<variable>`):
//! loading `societies.tsv`, the variable menu in the legend, and the HUD.
//! The layer shares the legend and the overlay's dots with the languages
//! layer, so the two are never on together.

use crate::ethnography::{self, Societies};
use crate::languages::{nearest, Dot};
use crate::{data, element, listen, App};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsValue;
use web_sys::{Document, HtmlSelectElement};

#[derive(Default)]
pub struct SocietiesState {
    pub on: bool,
    /// The variable shown, by id (kept from the URL until the data loads).
    pub variable: String,
    pub data: Option<Rc<Societies>>,
    requested: bool,
    pub dots: Vec<Dot>,
}

/// The variable shown when the layer opens.
pub const DEFAULT_VARIABLE: &str = "EA042";

/// `&societies=<variable id>` in a URL hash.
pub fn parse_hash(hash: &str) -> Option<String> {
    let v = hash
        .trim_start_matches('#')
        .split('&')
        .skip(1)
        .find_map(|kv| kv.strip_prefix("societies="))?;
    let ok = !v.is_empty() && v.len() <= 16 && v.chars().all(|c| c.is_ascii_alphanumeric());
    ok.then(|| v.to_string())
}

impl App {
    /// The index of the variable shown (the first if the id is unknown).
    fn society_variable(&self) -> Option<(Rc<Societies>, usize)> {
        let s = self.societies.data.clone()?;
        let v = s
            .variables
            .iter()
            .position(|v| v.id == self.societies.variable)
            .unwrap_or(0);
        Some((s, v))
    }

    pub(crate) fn set_societies(&mut self, on: bool) {
        self.societies.on = on;
        let _ = self
            .societies_button
            .set_attribute("aria-pressed", if on { "true" } else { "false" });
        if on {
            if self.langs.on {
                self.set_languages(false);
            }
            if self.societies.variable.is_empty() {
                self.societies.variable = DEFAULT_VARIABLE.into();
            }
            self.load_societies();
        } else {
            self.societies.dots.clear();
        }
        self.ethno_select.set_hidden(!on);
        self.sync_legend();
        self.credits_for_layer();
        self.redraw.mark_render();
        self.redraw.mark_hud();
        self.hash_stale = true;
    }

    pub(crate) fn set_society_variable(&mut self, id: &str) {
        self.societies.variable = id.to_string();
        self.ethno_select.set_value(id);
        self.sync_legend();
        self.redraw.mark_render();
        self.redraw.mark_hud();
        self.hash_stale = true;
    }

    /// The legend rows of the variable shown, while the layer is on.
    pub(crate) fn societies_legend(&self) -> Option<(String, Vec<(&'static str, String)>)> {
        if !self.societies.on {
            return None;
        }
        let (s, v) = self.society_variable()?;
        // The menu under the title names the variable.
        Some(("Societies".to_string(), s.legend(v)))
    }

    fn load_societies(&mut self) {
        if std::mem::replace(&mut self.societies.requested, true) {
            return;
        }
        let Some(app) = self.me.upgrade() else { return };
        wasm_bindgen_futures::spawn_local(async move {
            let parsed = match data::fetch("societies.tsv").await {
                Ok(bytes) => String::from_utf8(bytes)
                    .map_err(|e| JsValue::from_str(&e.to_string()))
                    .and_then(|t| ethnography::parse(&t).map_err(JsValue::from)),
                Err(e) => Err(e),
            };
            let mut a = app.borrow_mut();
            match parsed {
                Ok(s) => {
                    // The menu lists the variables of the file.
                    let sel = a.ethno_select.clone();
                    sel.set_inner_html("");
                    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                        for v in &s.variables {
                            if let Ok(o) = doc.create_element("option") {
                                let _ = o.set_attribute("value", &v.id);
                                o.set_text_content(Some(&v.title));
                                let _ = sel.append_child(&o);
                            }
                        }
                    }
                    a.societies.data = Some(Rc::new(s));
                    if let Some((s, v)) = a.society_variable() {
                        let id = s.variables[v].id.clone();
                        a.set_society_variable(&id);
                    }
                    a.redraw.mark_render();
                    a.set_globe_attr("data-societies", "1");
                }
                Err(e) => {
                    a.societies.requested = false;
                    web_sys::console::error_1(&e);
                }
            }
        });
    }

    /// The dots and names to draw this frame, while the layer is on.
    pub(crate) fn society_marks(&mut self) -> Option<crate::overlay::PointMarks> {
        if !self.societies.on {
            return None;
        }
        let (s, v) = self.society_variable()?;
        let dots = s.dots(&self.camera);
        let names = s.names(&self.camera, &dots, &[]);
        let marks = crate::overlay::PointMarks {
            dots: dots
                .iter()
                .map(|d| (d.x, d.y, s.color(v, d.index)))
                .collect(),
            names: names
                .into_iter()
                .map(|(i, x, y)| (x, y, s.list[i].name.clone()))
                .collect(),
        };
        self.societies.dots = dots;
        Some(marks)
    }

    /// HUD text for the society under the cursor, if any.
    pub(crate) fn society_at(&self, x: f64, y: f64) -> Option<String> {
        if !self.societies.on {
            return None;
        }
        let (s, v) = self.society_variable()?;
        let i = nearest(&self.societies.dots, x, y)?;
        Some(s.describe(v, i))
    }
}

/// The variable menu.
pub fn wire(app: &Rc<RefCell<App>>, doc: &Document) -> Result<(), JsValue> {
    let sel: HtmlSelectElement = element(doc, "ethno-var")?;
    let app = app.clone();
    let s = sel.clone();
    listen(sel.as_ref(), "change", true, move |_: web_sys::Event| {
        app.borrow_mut().set_society_variable(&s.value());
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_hash_names_the_variable() {
        assert_eq!(
            super::parse_hash("#1,2,3&societies=EA043"),
            Some("EA043".into())
        );
        assert_eq!(super::parse_hash("#1,2,3"), None);
        assert_eq!(super::parse_hash("#1,2,3&societies="), None);
        assert_eq!(super::parse_hash("#1,2,3&societies=<x>"), None);
    }
}
