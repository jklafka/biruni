//! DOM glue for the data credits panel (markup in `index.html`).
//!
//! The panel is a non-modal dialog opened by the HUD's "Data credits" button.
//! Opening moves focus to its close button; closing returns focus to the
//! button. All text goes in with `textContent`.

use crate::credits;
use crate::view::Layer;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, HtmlElement};

pub struct CreditsPanel {
    doc: Document,
    pub button: HtmlElement,
    panel: HtmlElement,
    pub close: HtmlElement,
    list: HtmlElement,
}

fn element(doc: &Document, id: &str) -> Result<HtmlElement, JsValue> {
    doc.get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("missing #{id}")))?
        .dyn_into::<HtmlElement>()
        .map_err(|_| JsValue::from_str(&format!("#{id} has the wrong element type")))
}

impl CreditsPanel {
    pub fn new(doc: &Document) -> Result<Self, JsValue> {
        Ok(Self {
            doc: doc.clone(),
            button: element(doc, "credits-button")?,
            panel: element(doc, "credits")?,
            close: element(doc, "credits-close")?,
            list: element(doc, "credits-list")?,
        })
    }

    pub fn is_open(&self) -> bool {
        !self.panel.hidden()
    }

    /// Rebuild the list for the shown layer.
    pub fn show_layer(&self, layer: Layer) {
        self.show(credits::visible_credits(layer));
    }

    /// Rebuild the list for the sky view (with the survey, if shown).
    pub fn show_sky(&self, survey: bool) {
        self.show(credits::sky_credits(survey));
    }

    pub fn show_solar(&self) {
        self.show(credits::solar_credits());
    }

    fn show(&self, list: Vec<&'static credits::Credit>) {
        self.list.set_text_content(None);
        for c in list {
            if let Err(e) = self.add_entry(c) {
                web_sys::console::error_1(&e);
            }
        }
    }

    pub fn set_open(&self, open: bool) {
        self.panel.set_hidden(!open);
        let _ = self
            .button
            .set_attribute("aria-expanded", if open { "true" } else { "false" });
        // Focus follows the panel: in on open, back to the button on close.
        let _ = if open {
            self.close.focus()
        } else {
            self.button.focus()
        };
    }

    /// True when `target` is inside the panel.
    pub fn contains(&self, target: &web_sys::EventTarget) -> bool {
        target
            .dyn_ref::<web_sys::Node>()
            .is_some_and(|n| self.panel.contains(Some(n)))
    }

    fn link(&self, text: &str, href: &str) -> Result<web_sys::Element, JsValue> {
        let a = self.doc.create_element("a")?;
        a.set_text_content(Some(text));
        a.set_attribute("href", href)?;
        a.set_attribute("target", "_blank")?;
        a.set_attribute("rel", "noopener noreferrer")?;
        Ok(a)
    }

    fn add_entry(&self, c: &credits::Credit) -> Result<(), JsValue> {
        let li = self.doc.create_element("li")?;
        li.set_attribute("data-credit", c.id)?;
        let name = self.doc.create_element("div")?;
        name.set_class_name("cr-name");
        let source = self.link(c.name, c.source_url)?;
        name.append_child(&source)?;
        let license = self.doc.create_element("div")?;
        let label = self.doc.create_element("span")?;
        label.set_text_content(Some("License: "));
        let terms = self.link(c.license, c.license_url)?;
        license.append_child(&label)?;
        license.append_child(&terms)?;
        let text = self.doc.create_element("div")?;
        text.set_class_name("cr-text");
        text.set_text_content(Some(c.text));
        li.append_child(&name)?;
        li.append_child(&license)?;
        li.append_child(&text)?;
        self.list.append_child(&li)?;
        Ok(())
    }
}
