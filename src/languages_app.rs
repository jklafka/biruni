//! The languages layer's part of the app (`W`, `&languages=1`): loading
//! `languages.tsv`, the legend, search entries and the HUD. Where the dots
//! go is `languages.rs` (browser-free); the overlay draws them.

use crate::languages::{self, Dot, Languages};
use crate::{credits, data, App};
use std::rc::Rc;
use wasm_bindgen::JsValue;

/// What the languages layer keeps.
#[derive(Default)]
pub struct LanguagesState {
    pub on: bool,
    pub data: Option<Rc<Languages>>,
    requested: bool,
    /// The dots last drawn, for hovering.
    pub dots: Vec<Dot>,
}

/// `&languages=1` in a URL hash.
pub fn parse_hash(hash: &str) -> bool {
    hash.trim_start_matches('#')
        .split('&')
        .skip(1)
        .any(|kv| kv == "languages=1")
}

impl App {
    /// The dots and names to draw this frame, while the layer is on and
    /// loaded; remembers the dots for hovering.
    pub(crate) fn language_marks(&mut self) -> Option<crate::overlay::PointMarks> {
        let data = self.langs.data.clone().filter(|_| self.langs.on)?;
        let dots = data.dots(&self.camera);
        let names = data.names(&self.camera, &dots, &[]);
        let marks = crate::overlay::PointMarks {
            dots: dots
                .iter()
                .map(|d| (d.x, d.y, data.color(d.index)))
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
        self.sync_legend();
        self.credits_for_layer();
        self.redraw.mark_render();
        self.redraw.mark_hud();
        self.hash_stale = true;
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
        match (&self.langs.data, self.langs.on) {
            (Some(l), true) => self.legend.show_rows(
                "Language families",
                &l.legend(),
                "Glottolog 5.4; dots at each language's location",
            ),
            _ if self.layers_state.shown() == crate::view::Layer::Density => {
                let rows = crate::view::density_legend();
                let rows: Vec<(&str, String)> =
                    rows.iter().map(|(c, t)| (c.as_str(), t.clone())).collect();
                self.legend.show_rows(
                    "People per km²",
                    &rows,
                    "GHS-POP 2020 (EC JRC), on a 10 km grid; dark: fewer than 0.1",
                );
            }
            _ => {
                let theme = self.layers_state.shown().theme();
                let year = self.countries.as_ref().map_or(0, |c| c.data_year());
                self.legend.show(theme, year);
            }
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
                    a.langs.data = Some(Rc::new(l));
                    a.sync_legend();
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
        Some(l.describe(i))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_hash_turns_languages_on() {
        assert!(super::parse_hash("#1,2,3&languages=1"));
        assert!(super::parse_hash("#1,2,3&year=1500&languages=1"));
        assert!(!super::parse_hash("#1,2,3"));
        assert!(!super::parse_hash("#languages=1"));
        assert!(!super::parse_hash("#1,2,3&languages=0"));
    }
}
