//! DOM glue for the thematic layers' legend (markup in `index.html`).
//!
//! The legend is a plain HTML list, not canvas, so screen readers read each
//! class and its range. The swatch colors come from `thematic::Theme::palette`,
//! the same table the shader gets, so they cannot drift apart.

use crate::thematic::{self, Theme};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, HtmlElement};

pub struct Legend {
    doc: Document,
    panel: HtmlElement,
    title: HtmlElement,
    list: HtmlElement,
    note: HtmlElement,
}

fn element(doc: &Document, id: &str) -> Result<HtmlElement, JsValue> {
    doc.get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("missing #{id}")))?
        .dyn_into::<HtmlElement>()
        .map_err(|_| JsValue::from_str(&format!("#{id} has the wrong element type")))
}

/// `[0x46, 0x32, 0x7e]` becomes `"#46327e"`.
fn css_color(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

impl Legend {
    pub fn new(doc: &Document) -> Result<Self, JsValue> {
        Ok(Self {
            doc: doc.clone(),
            panel: element(doc, "legend")?,
            title: element(doc, "legend-title")?,
            list: element(doc, "legend-list")?,
            note: element(doc, "legend-note")?,
        })
    }

    /// Show the legend of a theme, or hide it for `None`. `year` is the year
    /// most countries' estimates share (0 when unknown). Text goes in with
    /// `textContent`, never as markup.
    pub fn show(&self, theme: Option<Theme>, year: u16) {
        let Some(theme) = theme else {
            self.panel.set_hidden(true);
            return;
        };
        self.title.set_text_content(Some(theme.title()));
        self.note
            .set_text_content(Some(&thematic::source_note(year)));
        self.list.set_text_content(None);
        let palette = theme.palette();
        let rows = theme
            .legend()
            .into_iter()
            .map(|(class, text)| (palette[usize::from(class)], text))
            .chain(std::iter::once((
                thematic::NO_DATA_COLOR,
                "No data".to_owned(),
            )));
        for (color, text) in rows {
            if let Err(e) = self.add_row(color, &text) {
                web_sys::console::error_1(&e);
            }
        }
        self.panel.set_hidden(false);
    }

    fn add_row(&self, color: [u8; 3], text: &str) -> Result<(), JsValue> {
        let li = self.doc.create_element("li")?;
        let swatch = self.doc.create_element("span")?;
        swatch.set_class_name("sw");
        swatch.set_attribute("style", &format!("background:{}", css_color(color)))?;
        swatch.set_attribute("aria-hidden", "true")?;
        let label = self.doc.create_element("span")?;
        label.set_text_content(Some(text));
        li.append_child(&swatch)?;
        li.append_child(&label)?;
        self.list.append_child(&li)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::css_color;

    #[test]
    fn css_colors_are_lowercase_hex() {
        assert_eq!(css_color([0x46, 0x32, 0x7e]), "#46327e");
        assert_eq!(css_color([0, 0, 0]), "#000000");
        assert_eq!(css_color([255, 255, 255]), "#ffffff");
    }
}
