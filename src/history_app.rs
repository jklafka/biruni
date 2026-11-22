//! The historical borders view's part of the app (`H`, `&year=`): its year
//! panel, loading `history_borders.bin`, and swapping the political layer's
//! country IDs, colors, border lines and names for those of a year, then
//! back. The data and the per-year work are in `history.rs`
//! (browser-free).
//!
//! The swap reuses the Earth view's political machinery unchanged: the ID
//! raster (`u_ids`), the per-ID color table, the border line set and the
//! label layer. Whatever loads later (the map data, the countries, the
//! labels) and a restored WebGL context put the modern data back, so each
//! of those calls [`App::history_reapply`].

use crate::history::{self, Timeline};
use crate::scrub::Stepper;
use crate::view::Layer;
use crate::{data, element, listen, App};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsValue;
use web_sys::{Document, HtmlElement, HtmlInputElement, MouseEvent};

/// The year panel (markup in `index.html`).
pub struct YearUi {
    panel: HtmlElement,
    range: HtmlInputElement,
    value: HtmlElement,
    button: HtmlElement,
    play: PlayUi,
}

/// Play rates of the year panel: years a second, and the speed button's text.
pub const YEAR_RATES: [(f64, &str); 3] =
    [(10.0, "10 yr/s"), (50.0, "50 yr/s"), (250.0, "250 yr/s")];

/// A panel's play and speed buttons (the year and age panels).
pub struct PlayUi {
    play: HtmlElement,
    speed: HtmlElement,
}

impl PlayUi {
    pub fn new(doc: &Document, play: &str, speed: &str) -> Result<PlayUi, JsValue> {
        Ok(PlayUi {
            play: element(doc, play)?,
            speed: element(doc, speed)?,
        })
    }

    pub fn show(&self, s: &Stepper, speed_text: &str) {
        let (on, label) = if s.on {
            ("true", "Pause")
        } else {
            ("false", "Play")
        };
        let _ = self.play.set_attribute("aria-pressed", on);
        let _ = self.play.set_attribute("aria-label", label);
        let _ = self.play.set_attribute("title", &format!("{label} (T)"));
        self.play.set_text_content(Some(if s.on {
            "\u{23F8}\u{FE0E}"
        } else {
            "\u{25B6}\u{FE0E}"
        }));
        self.speed.set_text_content(Some(speed_text));
        let _ = self
            .speed
            .set_attribute("aria-label", &format!("Play speed: {speed_text}"));
    }
}

impl YearUi {
    pub fn new(doc: &Document) -> Result<YearUi, JsValue> {
        Ok(YearUi {
            panel: element(doc, "years")?,
            range: element(doc, "year-range")?,
            value: element(doc, "year-val")?,
            button: element(doc, "history-toggle")?,
            play: PlayUi::new(doc, "year-play", "year-speed")?,
        })
    }

    fn show(&self, on: bool, year: i64) {
        self.panel.set_hidden(!on);
        let _ = self
            .button
            .set_attribute("aria-pressed", if on { "true" } else { "false" });
        // The slider counts astronomically (1 BCE = 0), so it has no gap.
        let astro = if year < 0 { year + 1 } else { year };
        self.range.set_value(&astro.to_string());
        let text = history::format_year(year);
        self.value.set_text_content(Some(&text));
        let _ = self.range.set_attribute("aria-valuetext", &text);
    }
}

/// What the historical view keeps.
#[derive(Default)]
pub struct HistoryState {
    pub on: bool,
    pub year: i64,
    data: Option<Rc<Timeline>>,
    requested: bool,
    /// The active records and their ID raster, as last applied.
    applied: Option<Vec<usize>>,
    ids: Vec<u8>,
    ids_width: usize,
    /// The year changed (or the modern data came back): apply next frame.
    dirty: bool,
    /// The layer to return to.
    prev_layer: Option<Layer>,
    /// The modern place names, to put back.
    pub modern_labels: Option<Vec<crate::labels::Label>>,
    /// Play through the years.
    pub play: Stepper,
}

impl HistoryState {
    pub fn new(year: Option<i64>) -> HistoryState {
        HistoryState {
            on: year.is_some(),
            year: year.unwrap_or(history::DEFAULT_YEAR),
            play: Stepper::new(1),
            ..HistoryState::default()
        }
    }
}

impl App {
    /// Show or hide the historical borders.
    pub(crate) fn set_history(&mut self, on: bool) {
        let was = std::mem::replace(&mut self.history.on, on);
        if let Some(body) = self.window.document().and_then(|d| d.body()) {
            let _ = body.class_list().toggle_with_force("history-on", on);
        }
        self.year_ui.show(on, self.history.year);
        if on && !was {
            if self.paleo.on {
                self.set_paleo(false);
            }
            // The terminator has no place here; the political style does.
            if self.sun_on {
                self.toggle_sun();
            }
            self.select(None);
            let layer = self.layers_state.wanted();
            self.history.prev_layer = Some(layer);
            if layer != Layer::Political {
                self.select_layer(Layer::Political);
            }
            self.load_history();
            self.history_reapply();
        }
        if !on && was {
            self.set_year_playing(false);
            self.restore_modern();
            if let Some(l) = self.history.prev_layer.take() {
                self.select_layer(l);
            }
        }
        self.credits_for_layer();
        self.sync_legend();
        self.update_hover();
        self.redraw.mark_render();
        self.redraw.mark_hud();
        self.hash_stale = true;
    }

    /// Credits for what the Earth view shows now.
    pub(crate) fn credits_for_layer(&mut self) {
        self.credits.show_list(self.earth_credits());
    }

    pub(crate) fn set_year(&mut self, year: i64) {
        self.history.year = history::clamp_year(year);
        self.year_ui.show(self.history.on, self.history.year);
        self.history.dirty = true;
        self.redraw.mark_render();
        self.hash_stale = true;
    }

    /// Step the year by `n` (skipping year 0).
    pub(crate) fn step_year(&mut self, n: i64) {
        let y = history::add_years(self.history.year, n);
        self.set_year(y);
    }

    /// Play or pause the years. Play from the last year starts over at the
    /// first.
    pub(crate) fn set_year_playing(&mut self, on: bool) {
        if on && self.history.year >= history::YEAR_MAX {
            self.set_year(history::YEAR_MIN);
        }
        self.history.play.set_on(on);
        self.show_year_play();
    }

    pub(crate) fn cycle_year_speed(&mut self) {
        self.history.play.cycle(YEAR_RATES.len());
        self.show_year_play();
    }

    fn show_year_play(&self) {
        let speed = YEAR_RATES[self.history.play.speed % YEAR_RATES.len()].1;
        self.year_ui.play.show(&self.history.play, speed);
    }

    /// Called every Earth frame: while playing, move the year on; stop at
    /// the last year.
    pub(crate) fn history_play(&mut self, dt_ms: f64) {
        if !self.history.on {
            return;
        }
        let rate = YEAR_RATES[self.history.play.speed % YEAR_RATES.len()].0;
        let n = self.history.play.steps(dt_ms, rate);
        if n == 0 {
            return;
        }
        self.set_year(history::add_years(self.history.year, n));
        if self.history.year >= history::YEAR_MAX {
            self.set_year_playing(false);
        }
    }

    /// Apply the year again at the next frame (after the modern data was
    /// put back by a load or a context restore).
    pub(crate) fn history_reapply(&mut self) {
        // Until the year is drawn again, tests must not take the modern
        // data that was just put back for it.
        let _ = self.canvas.remove_attribute("data-year");
        self.history.applied = None;
        self.history.dirty = true;
        self.redraw.mark_render();
    }

    fn load_history(&mut self) {
        if std::mem::replace(&mut self.history.requested, true) {
            return;
        }
        let Some(app) = self.me.upgrade() else { return };
        wasm_bindgen_futures::spawn_local(async move {
            let t = match data::fetch("history_borders.bin").await {
                Ok(bytes) => history::decode(&bytes).map_err(JsValue::from),
                Err(e) => Err(e),
            };
            let mut a = app.borrow_mut();
            match t {
                Ok(t) => {
                    a.history.data = Some(Rc::new(t));
                    a.history_reapply();
                }
                Err(e) => {
                    a.history.requested = false;
                    web_sys::console::error_1(&e);
                }
            }
        });
    }

    /// Called every Earth frame: if the year (or what is on the GPU)
    /// changed, swap in that year's IDs, colors, borders and names. Only
    /// a change of the active records does any work.
    pub(crate) fn history_frame(&mut self) {
        if !(self.history.on && self.history.dirty) {
            return;
        }
        let Some(t) = self.history.data.clone() else {
            return;
        };
        self.history.dirty = false;
        let active = t.active(self.history.year);
        if self.history.applied.as_ref() == Some(&active) {
            return;
        }
        let width = self.renderer.ids_width();
        let ids = t.rasterize(&active, width);
        if let Err(e) = self.renderer.upload_country_ids(&ids, width) {
            web_sys::console::error_1(&e);
        }
        self.renderer.set_country_colors(&t.id_colors(&active));
        let levels = crate::lines::build_levels(&t.border_rings(&active));
        if let Err(e) = self.renderer.set_border_lines(&levels) {
            web_sys::console::error_1(&e);
        }
        self.labels.set_labels(t.labels(&active));
        self.history.ids = ids;
        self.history.ids_width = width;
        self.history.applied = Some(active);
        self.redraw.mark_render();
        self.redraw.mark_hud();
        // Lets the visual tests wait for the year to be drawn.
        self.set_globe_attr("data-year", &self.history.year.to_string());
    }

    /// Put the modern countries, borders and names back.
    pub(crate) fn restore_modern(&mut self) {
        let width = self.renderer.ids_width();
        if let Some(ids) = self.ids.clone() {
            if let Err(e) = self.renderer.upload_country_ids(&ids, width) {
                web_sys::console::error_1(&e);
            }
        }
        if let Some(c) = &self.countries {
            self.renderer.set_country_colors(&c.id_colors());
        }
        if let Some(layers) = self.layers.clone() {
            if let Err(e) = self.renderer.set_border_lines(&layers.lines().borders) {
                web_sys::console::error_1(&e);
            }
        }
        if let Some(l) = self.history.modern_labels.clone() {
            self.labels.set_labels(l);
        }
        self.history.applied = None;
        self.history.ids.clear();
        let _ = self.canvas.remove_attribute("data-year");
    }

    /// HUD text for the polity under a lon/lat (degrees), if any.
    pub(crate) fn history_at(&self, lat: f64, lon: f64) -> Option<String> {
        let t = self.history.data.as_ref()?;
        let active = self.history.applied.as_ref()?;
        let ri = t.record_at(active, &self.history.ids, self.history.ids_width, lat, lon)?;
        t.describe(ri)
    }
}

/// The year panel's slider and buttons.
pub fn wire(app: &Rc<RefCell<App>>, doc: &Document) -> Result<(), JsValue> {
    let range: HtmlInputElement = element(doc, "year-range")?;
    {
        let app = app.clone();
        let r = range.clone();
        listen(range.as_ref(), "input", true, move |_: web_sys::Event| {
            if let Ok(astro) = r.value().parse::<i64>() {
                // Back from the gapless slider value (0 = 1 BCE).
                let year = if astro <= 0 { astro - 1 } else { astro };
                app.borrow_mut().set_year(year);
            }
        })?;
    }
    for (id, step) in [
        ("year-back", -100),
        ("year-prev", -1),
        ("year-next", 1),
        ("year-fwd", 100),
    ] {
        let button: HtmlElement = element(doc, id)?;
        let app = app.clone();
        listen(button.as_ref(), "click", true, move |_: MouseEvent| {
            app.borrow_mut().step_year(step)
        })?;
    }
    {
        let button: HtmlElement = element(doc, "year-play")?;
        let app = app.clone();
        listen(button.as_ref(), "click", true, move |_: MouseEvent| {
            let mut a = app.borrow_mut();
            let on = !a.history.play.on;
            a.set_year_playing(on);
        })?;
    }
    let button: HtmlElement = element(doc, "year-speed")?;
    let app = app.clone();
    listen(button.as_ref(), "click", true, move |_: MouseEvent| {
        app.borrow_mut().cycle_year_speed()
    })
}
