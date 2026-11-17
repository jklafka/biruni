//! DOM glue for the country info panel (markup lives in `index.html`).

use crate::country::Country;
use crate::thematic::Theme;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, HtmlElement};

/// Rows pre-built in `index.html` (`#ci-r0` ... with `#ci-l0` / `#ci-v0`).
const ROWS: usize = 5;

pub struct InfoPanel {
    panel: HtmlElement,
    title: HtmlElement,
    rows: Vec<(HtmlElement, HtmlElement, HtmlElement)>,
}

fn element(doc: &Document, id: &str) -> Result<HtmlElement, JsValue> {
    doc.get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("missing #{id}")))?
        .dyn_into::<HtmlElement>()
        .map_err(|_| JsValue::from_str(&format!("#{id} has the wrong element type")))
}

impl InfoPanel {
    pub fn new(doc: &Document) -> Result<Self, JsValue> {
        let mut rows = Vec::new();
        for i in 0..ROWS {
            rows.push((
                element(doc, &format!("ci-r{i}"))?,
                element(doc, &format!("ci-l{i}"))?,
                element(doc, &format!("ci-v{i}"))?,
            ));
        }
        Ok(Self {
            panel: element(doc, "country-info")?,
            title: element(doc, "ci-title")?,
            rows,
        })
    }

    /// Show a country's name and attributes, or hide the panel for `None`.
    /// On a thematic layer its value comes first. Text goes in with
    /// `textContent`, never as markup.
    pub fn show(&self, country: Option<&Country>, theme: Option<Theme>) {
        let Some(c) = country else {
            self.panel.set_hidden(true);
            return;
        };
        let title = match c.flag() {
            Some(flag) => format!("{flag} {}", c.name),
            None => c.name.clone(),
        };
        self.title.set_text_content(Some(&title));
        let mut data = c.rows();
        if let Some(t) = theme {
            data.insert(0, (t.title(), t.describe(&c.stats)));
        }
        for (i, (row, label, value)) in self.rows.iter().enumerate() {
            match data.get(i) {
                Some((l, v)) => {
                    label.set_text_content(Some(l));
                    value.set_text_content(Some(v));
                    row.set_hidden(false);
                }
                None => row.set_hidden(true),
            }
        }
        self.panel.set_hidden(false);
    }
}
