//! Share button: Web Share API where available, otherwise the clipboard,
//! otherwise a text field the user can copy from by hand.
//!
//! Nothing here touches the render loop, so sharing never redraws the globe.

use js_sys::{Function, Object, Promise, Reflect};
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Document, HtmlElement, HtmlInputElement, KeyboardEvent, Window};

const TOAST_MS: i32 = 2200;

pub struct ShareUi {
    window: Window,
    canvas: HtmlElement,
    toast: HtmlElement,
    panel: HtmlElement,
    input: HtmlInputElement,
    timer: Cell<Option<i32>>,
}

enum Outcome {
    Done,
    Cancelled,
    Failed,
}

impl ShareUi {
    pub fn new(window: &Window, doc: &Document, canvas: HtmlElement) -> Result<Rc<Self>, JsValue> {
        let ui = Rc::new(ShareUi {
            window: window.clone(),
            canvas,
            toast: crate::element(doc, "toast")?,
            panel: crate::element(doc, "share-fallback")?,
            input: crate::element(doc, "share-url")?,
            timer: Cell::new(None),
        });
        // Escape closes the fallback; other keys must not reach the globe
        // shortcuts while the user is selecting or copying text.
        {
            let ui2 = ui.clone();
            crate::listen(
                ui.input.as_ref(),
                "keydown",
                true,
                move |e: KeyboardEvent| {
                    e.stop_propagation();
                    if e.key() == "Escape" {
                        ui2.hide_panel();
                        let _ = ui2.canvas.focus();
                    }
                },
            )?;
        }
        {
            let ui2 = ui.clone();
            crate::listen(ui.input.as_ref(), "blur", true, move |_: web_sys::Event| {
                ui2.hide_panel()
            })?;
        }
        Ok(ui)
    }

    fn hide_panel(&self) {
        self.panel.set_hidden(true);
    }

    fn toast(&self, text: &str) {
        if let Some(id) = self.timer.take() {
            self.window.clear_timeout_with_handle(id);
        }
        self.toast.set_text_content(Some(text));
        self.toast.set_hidden(false);
        let toast = self.toast.clone();
        let hide = Closure::once_into_js(move || toast.set_hidden(true));
        let id = self
            .window
            .set_timeout_with_callback_and_timeout_and_arguments_0(hide.unchecked_ref(), TOAST_MS)
            .ok();
        self.timer.set(id);
    }

    /// Manual fallback: show the link selected in a text field.
    fn show_panel(&self, url: &str) {
        self.input.set_value(url);
        self.panel.set_hidden(false);
        let _ = self.input.focus();
        self.input.select();
    }
}

/// Share `url`. Call from a click or key handler (needs a user gesture) and
/// do not hold any `RefCell` borrow across it.
pub fn share(ui: &Rc<ShareUi>, url: String) {
    let ui = ui.clone();
    wasm_bindgen_futures::spawn_local(async move {
        match try_web_share(&ui.window, &url).await {
            Outcome::Done | Outcome::Cancelled => return,
            Outcome::Failed => {}
        }
        match try_clipboard(&ui.window, &url).await {
            Outcome::Done => ui.toast("Link copied"),
            _ => {
                ui.show_panel(&url);
                ui.toast("Copy the link below");
            }
        }
    });
}

fn method(target: &JsValue, name: &str) -> Option<Function> {
    Reflect::get(target, &JsValue::from_str(name))
        .ok()?
        .dyn_into::<Function>()
        .ok()
}

async fn try_web_share(window: &Window, url: &str) -> Outcome {
    let nav: JsValue = window.navigator().into();
    let Some(share) = method(&nav, "share") else {
        return Outcome::Failed;
    };
    let data = Object::new();
    let _ = Reflect::set(&data, &"url".into(), &url.into());
    let _ = Reflect::set(&data, &"title".into(), &"Maps".into());
    let promise = match share.call1(&nav, &data) {
        Ok(p) => p,
        Err(_) => return Outcome::Failed,
    };
    match JsFuture::from(Promise::resolve(&promise)).await {
        Ok(_) => Outcome::Done,
        Err(e) => {
            // Dismissing the share sheet is not an error.
            let name = Reflect::get(&e, &"name".into())
                .ok()
                .and_then(|n| n.as_string());
            if name.as_deref() == Some("AbortError") {
                Outcome::Cancelled
            } else {
                Outcome::Failed
            }
        }
    }
}

async fn try_clipboard(window: &Window, url: &str) -> Outcome {
    let nav: JsValue = window.navigator().into();
    let Ok(clipboard) = Reflect::get(&nav, &"clipboard".into()) else {
        return Outcome::Failed;
    };
    let Some(write) = method(&clipboard, "writeText") else {
        return Outcome::Failed;
    };
    let promise = match write.call1(&clipboard, &url.into()) {
        Ok(p) => p,
        Err(_) => return Outcome::Failed,
    };
    match JsFuture::from(Promise::resolve(&promise)).await {
        Ok(_) => Outcome::Done,
        Err(_) => Outcome::Failed,
    }
}
