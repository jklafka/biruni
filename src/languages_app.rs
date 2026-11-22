//! The languages layer's part of the app (`W`, `&languages=1`, or
//! `&languages=<variable>` colored by endangerment or a Grambank feature):
//! loading `languages.tsv`, the "Color by" menu in the legend, search
//! entries and the HUD. Where the dots go is `languages.rs`
//! (browser-free); the overlay draws them.

use crate::languages::{self, Dot, Languages};
use crate::{credits, data, element, listen, App};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsValue;
use web_sys::{Document, HtmlSelectElement};

/// What the languages layer keeps.
#[derive(Default)]
pub struct LanguagesState {
    pub on: bool,
    pub data: Option<Rc<Languages>>,
    requested: bool,
    /// The dots last drawn, for hovering.
    pub dots: Vec<Dot>,
    /// The variable the dots are colored by, by id; empty for families
    /// (kept from the URL until the data loads).
    pub by: String,
}

/// Legend rows: (CSS color, label).
type LegendRows = Vec<(&'static str, String)>;

/// The "Color by" menu's value for families.
const FAMILY: &str = "family";

/// `&languages=1` (by family) or `&languages=<variable id>` in a URL hash:
/// the variable id, empty for families.
pub fn parse_hash(hash: &str) -> Option<String> {
    let v = hash
        .trim_start_matches('#')
        .split('&')
        .skip(1)
        .find_map(|kv| kv.strip_prefix("languages="))?;
    if v == "1" {
        return Some(String::new());
    }
    let ok = !v.is_empty() && v.len() <= 16 && v.chars().all(|c| c.is_ascii_alphanumeric());
    ok.then(|| v.to_string())
}

/// The hash value for a variable id (see [`parse_hash`]).
pub fn hash_value(by: &str) -> &str {
    if by.is_empty() {
        "1"
    } else {
        by
    }
}

impl App {
    /// The dots and names to draw this frame, while the layer is on and
    /// loaded; remembers the dots for hovering.
    pub(crate) fn language_marks(&mut self) -> Option<crate::overlay::PointMarks> {
        let data = self.langs.data.clone().filter(|_| self.langs.on)?;
        let by = self.language_by();
        let dots = data.dots(&self.camera, by);
        let names = data.names(&self.camera, &dots, &[]);
        let marks = crate::overlay::PointMarks {
            dots: dots
                .iter()
                .filter_map(|d| Some((d.x, d.y, data.color(by, d.index)?)))
                .collect(),
            names: names
                .into_iter()
                .map(|(i, x, y)| (x, y, data.list[i].name.clone()))
                .collect(),
        };
        self.langs.dots = dots;
        Some(marks)
    }

    pub(crate) fn set_languages(&mut self, on: bool) {
        self.langs.on = on;
        let _ = self
            .langs_button
            .set_attribute("aria-pressed", if on { "true" } else { "false" });
        if on {
            if self.societies.on {
                self.set_societies(false);
            }
            self.load_languages();
        } else {
            self.langs.dots.clear();
        }
        self.lang_select
            .set_hidden(!on || self.langs.data.is_none());
        self.sync_legend();
        self.credits_for_layer();
        self.redraw.mark_render();
        self.redraw.mark_hud();
        self.hash_stale = true;
    }

    /// The variable the dots are colored by (`None`: families, also for an
    /// unknown id).
    pub(crate) fn language_by(&self) -> Option<usize> {
        self.langs.data.as_ref()?.variable(&self.langs.by)
    }

    /// Color the dots by a variable id ("" or "family" for families).
    pub(crate) fn set_language_by(&mut self, id: &str) {
        self.langs.by = if id == FAMILY {
            String::new()
        } else {
            id.to_string()
        };
        if self.langs.data.is_some() && self.language_by().is_none() {
            self.langs.by.clear();
        }
        let shown = if self.langs.by.is_empty() {
            FAMILY
        } else {
            &self.langs.by
        };
        self.lang_select.set_value(shown);
        self.sync_legend();
        self.redraw.mark_render();
        self.redraw.mark_hud();
        self.hash_stale = true;
    }

    /// The legend title, rows and note of the languages layer.
    fn languages_legend(&self) -> Option<(&'static str, LegendRows, String)> {
        let l = self.langs.data.as_ref().filter(|_| self.langs.on)?;
        let by = self.language_by();
        let rows = l.legend(by);
        let Some(v) = by else {
            return Some((
                "Languages",
                rows,
                "Glottolog 5.4; dots at each language's location".into(),
            ));
        };
        let var = &l.variables[v];
        let source = if var.id == languages::ENDANGERMENT {
            "Glottolog 5.4".to_string()
        } else {
            format!("Grambank 1.0.3 ({}): {}", var.id, var.question)
        };
        let source = source.trim_end_matches('.');
        let sep = if source.ends_with('?') { "" } else { "." };
        let note = format!(
            "{source}{sep} {} languages without data are not shown.",
            l.without_data(v)
        );
        Some(("Languages", rows, note))
    }

    /// The legend: the languages' families while that layer is on, else
    /// the thematic layer's (if any).
    pub(crate) fn sync_legend(&mut self) {
        if let Some((title, rows)) = self.societies_legend() {
            self.legend.show_rows(
                &title,
                &rows,
                "D-PLACE: Ethnographic Atlas (Murdock et al.)",
            );
            return;
        }
        if let Some((title, rows, note)) = self.languages_legend() {
            self.legend.show_rows(title, &rows, &note);
            return;
        }
        if self.layers_state.shown() == crate::view::Layer::Density {
            let rows = crate::view::density_legend();
            let rows: Vec<(&str, String)> =
                rows.iter().map(|(c, t)| (c.as_str(), t.clone())).collect();
            self.legend.show_rows(
                "People per km²",
                &rows,
                "GHS-POP 2020 (EC JRC), on a 10 km grid; dark: fewer than 0.1",
            );
        } else {
            let theme = self.layers_state.shown().theme();
            let year = self.countries.as_ref().map_or(0, |c| c.data_year());
            self.legend.show(theme, year);
        }
    }

    /// The Earth view's credits with the overlays that are on.
    pub(crate) fn earth_credits(&self) -> Vec<&'static credits::Credit> {
        let base = if self.paleo.on {
            credits::paleo_credits()
        } else if self.history.on {
            credits::history_credits()
        } else {
            credits::visible_credits(self.layers_state.shown())
        };
        credits::with_overlays(base, self.langs.on, self.societies.on)
    }

    fn load_languages(&mut self) {
        if std::mem::replace(&mut self.langs.requested, true) {
            return;
        }
        let Some(app) = self.me.upgrade() else { return };
        wasm_bindgen_futures::spawn_local(async move {
            let parsed = match data::fetch("languages.tsv").await {
                Ok(bytes) => String::from_utf8(bytes)
                    .map_err(|e| JsValue::from_str(&e.to_string()))
                    .and_then(|t| languages::parse(&t).map_err(JsValue::from)),
                Err(e) => Err(e),
            };
            let mut a = app.borrow_mut();
            match parsed {
                Ok(l) => {
                    if let Some(s) = &a.search {
                        s.add(l.places());
                    }
                    // The menu: families, then the file's variables.
                    let sel = a.lang_select.clone();
                    sel.set_inner_html("");
                    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                        let options = std::iter::once((FAMILY, "Color by family")).chain(
                            l.variables
                                .iter()
                                .map(|v| (v.id.as_str(), v.title.as_str())),
                        );
                        for (id, title) in options {
                            if let Ok(o) = doc.create_element("option") {
                                let _ = o.set_attribute("value", id);
                                o.set_text_content(Some(title));
                                let _ = sel.append_child(&o);
                            }
                        }
                    }
                    a.langs.data = Some(Rc::new(l));
                    let by = a.langs.by.clone();
                    a.set_language_by(&by);
                    let on = a.langs.on;
                    a.lang_select.set_hidden(!on);
                    a.redraw.mark_render();
                    a.set_globe_attr("data-languages", "1");
                }
                Err(e) => {
                    a.langs.requested = false;
                    web_sys::console::error_1(&e);
                }
            }
        });
    }

    /// HUD text for the language under the cursor, if any.
    pub(crate) fn language_at(&self, x: f64, y: f64) -> Option<String> {
        let l = self.langs.data.as_ref().filter(|_| self.langs.on)?;
        let i = l.hit(&self.langs.dots, x, y)?;
        Some(l.describe(self.language_by(), i))
    }
}

/// The "Color by" menu.
pub fn wire(app: &Rc<RefCell<App>>, doc: &Document) -> Result<(), JsValue> {
    let sel: HtmlSelectElement = element(doc, "lang-var")?;
    let app = app.clone();
    let s = sel.clone();
    listen(sel.as_ref(), "change", true, move |_: web_sys::Event| {
        app.borrow_mut().set_language_by(&s.value());
    })
}

#[cfg(test)]
mod tests {
    use super::{hash_value, parse_hash};

    #[test]
    fn the_hash_turns_languages_on() {
        assert_eq!(parse_hash("#1,2,3&languages=1"), Some(String::new()));
        assert_eq!(
            parse_hash("#1,2,3&year=1500&languages=1"),
            Some(String::new())
        );
        assert_eq!(parse_hash("#1,2,3&languages=aes"), Some("aes".into()));
        assert_eq!(parse_hash("#1,2,3&languages=GB020"), Some("GB020".into()));
        assert_eq!(parse_hash("#1,2,3"), None);
        assert_eq!(parse_hash("#languages=1"), None);
        assert_eq!(parse_hash("#1,2,3&languages="), None);
        assert_eq!(parse_hash("#1,2,3&languages=<x>"), None);
        assert_eq!(hash_value(""), "1");
        assert_eq!(hash_value("GB020"), "GB020");
    }
}
