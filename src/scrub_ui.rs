//! DOM glue for the time scrubber panel (markup lives in `index.html`).
//! The math is in `scrub.rs`; this only reads and writes the controls.

use crate::scrub::{self, PlayMode};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, HtmlElement, HtmlInputElement};

pub struct ScrubUi {
    panel: HtmlElement,
    date: HtmlInputElement,
    date_val: HtmlElement,
    time: HtmlInputElement,
    time_val: HtmlElement,
    play: HtmlElement,
    mode: HtmlElement,
    now: HtmlElement,
    local: HtmlElement,
}

fn get<T: JsCast>(doc: &Document, id: &str) -> Result<T, JsValue> {
    doc.get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("missing #{id}")))?
        .dyn_into::<T>()
        .map_err(|_| JsValue::from_str(&format!("#{id} has the wrong element type")))
}

/// Minutes east of UTC at `t`, from the browser's time zone.
fn local_offset_min(t: f64) -> f64 {
    -js_sys::Date::new(&JsValue::from_f64(t * 1000.0)).get_timezone_offset()
}

impl ScrubUi {
    pub fn new(doc: &Document) -> Result<Self, JsValue> {
        Ok(Self {
            panel: get(doc, "scrubber")?,
            date: get(doc, "scrub-date")?,
            date_val: get(doc, "scrub-date-val")?,
            time: get(doc, "scrub-time")?,
            time_val: get(doc, "scrub-time-val")?,
            play: get(doc, "scrub-play")?,
            mode: get(doc, "scrub-mode")?,
            now: get(doc, "scrub-now")?,
            local: get(doc, "scrub-local")?,
        })
    }

    pub fn time_input(&self) -> &HtmlInputElement {
        &self.time
    }
    pub fn date_input(&self) -> &HtmlInputElement {
        &self.date
    }
    pub fn play_button(&self) -> &HtmlElement {
        &self.play
    }
    pub fn mode_button(&self) -> &HtmlElement {
        &self.mode
    }
    pub fn now_button(&self) -> &HtmlElement {
        &self.now
    }

    pub fn show(&self, on: bool) {
        self.panel.set_hidden(!on);
    }

    /// Slider position of the time-of-day slider, in minutes.
    pub fn minute(&self) -> u32 {
        self.time.value().parse().unwrap_or(0)
    }

    /// Slider position of the day-of-year slider.
    pub fn day(&self) -> u32 {
        self.date.value().parse().unwrap_or(1)
    }

    /// Show time `t`. `live` follows the clock: the Now button is pressed.
    pub fn sync(&self, t: f64, live: bool, playing: bool, mode: PlayMode) {
        let max = scrub::year_length_of(t).to_string();
        if self.date.max() != max {
            self.date.set_max(&max);
        }
        let minute = scrub::minute_of_day(t);
        self.time.set_value(&minute.to_string());
        self.date.set_value(&scrub::day_of_year(t).to_string());
        self.time_val
            .set_text_content(Some(&scrub::format_clock(i64::from(minute) * 60)));
        self.date_val.set_text_content(Some(&scrub::format_date(t)));
        let local = format!("Local {}", scrub::format_local(t, local_offset_min(t)));
        self.local.set_text_content(Some(&local));
        let on = |b: bool| if b { "true" } else { "false" };
        let _ = self.now.set_attribute("aria-pressed", on(live));
        let _ = self.play.set_attribute("aria-pressed", on(playing));
        let _ = self
            .play
            .set_attribute("aria-label", if playing { "Pause" } else { "Play" });
        self.play
            .set_text_content(Some(if playing { "\u{23F8}" } else { "\u{25B6}" }));
        self.mode.set_text_content(Some(mode.label()));
        let _ = self.mode.set_attribute(
            "aria-label",
            &format!("Play mode: {}", mode.label().to_lowercase()),
        );
    }
}
