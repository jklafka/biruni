//! DOM wiring for place search: the search box, its results list and the
//! `/` shortcut. The matching itself lives in the browser-free `search`
//! module.
//!
//! The gazetteer (`places.tsv`) is fetched the first time the box gets
//! focus, never at startup. While typing, keystrokes stay inside the input:
//! `main`'s global key handler ignores events whose target is an input.

use crate::search::{highlight_segments, Gazetteer, Kind, Place};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Document, Element, EventTarget, HtmlElement, HtmlInputElement, KeyboardEvent};

/// Most results listed.
const MAX_RESULTS: usize = 6;

#[derive(PartialEq)]
enum Load {
    Idle,
    Loading,
    Done,
    Failed,
}

struct State {
    doc: Document,
    input: HtmlInputElement,
    list: HtmlElement,
    status: HtmlElement,
    gaz: Option<Gazetteer>,
    load: Load,
    results: Vec<Place>,
    active: usize,
    on_select: Rc<dyn Fn(f64, f64, f64)>,
}

type Shared = Rc<RefCell<State>>;

fn is_typing_target(e: &KeyboardEvent) -> bool {
    e.target().is_some_and(|t| {
        t.is_instance_of::<HtmlInputElement>() || t.is_instance_of::<web_sys::HtmlTextAreaElement>()
    })
}

impl State {
    fn set_status(&self, text: &str) {
        self.status.set_text_content(Some(text));
        self.status.set_hidden(text.is_empty());
    }

    fn hide_list(&mut self) {
        self.list.set_hidden(true);
        self.list.set_text_content(None);
        self.results.clear();
        self.active = 0;
        let _ = self.input.set_attribute("aria-expanded", "false");
        let _ = self.input.remove_attribute("aria-activedescendant");
        self.set_status("");
    }

    /// Recompute results for the current text and redraw the list.
    fn refresh(&mut self) {
        let text = self.input.value();
        self.results.clear();
        self.active = 0;
        if text.trim().is_empty() {
            self.hide_list();
            return;
        }
        match (&self.gaz, &self.load) {
            (Some(g), _) => {
                self.results = g.search(&text, MAX_RESULTS).into_iter().cloned().collect();
                let msg = if self.results.is_empty() {
                    "No matching places"
                } else {
                    ""
                };
                self.set_status(msg);
            }
            (None, Load::Failed) => self.set_status("Could not load place data"),
            (None, _) => self.set_status("Loading places…"),
        }
        self.render();
    }

    fn render(&mut self) {
        self.list.set_text_content(None);
        let query = self.input.value();
        for (i, p) in self.results.iter().enumerate() {
            let Ok(li) = self.doc.create_element("li") else {
                continue;
            };
            let _ = li.set_attribute("role", "option");
            let _ = li.set_attribute("id", &format!("search-opt-{i}"));
            let _ = li.set_attribute("data-index", &i.to_string());
            let name = self.doc.create_element("span");
            let sub = self.doc.create_element("span");
            if let (Ok(name), Ok(sub)) = (name, sub) {
                // Built from text nodes and <mark> elements, never innerHTML.
                for (text, hit) in highlight_segments(&p.name, &query) {
                    if let Ok(piece) = self.doc.create_element(if hit { "mark" } else { "span" }) {
                        piece.set_text_content(Some(text));
                        let _ = name.append_child(&piece);
                    }
                }
                let _ = name.set_attribute("class", "search-name");
                sub.set_text_content(Some(match p.kind {
                    Kind::Country => "Country",
                    Kind::Place => &p.country,
                }));
                let _ = sub.set_attribute("class", "search-sub");
                let _ = li.append_child(&name);
                let _ = li.append_child(&sub);
            }
            let _ = self.list.append_child(&li);
        }
        let open = !self.results.is_empty();
        self.list.set_hidden(!open);
        let _ = self
            .input
            .set_attribute("aria-expanded", if open { "true" } else { "false" });
        self.mark_active();
    }

    fn mark_active(&self) {
        let children = self.list.children();
        for i in 0..children.length() {
            if let Some(li) = children.item(i) {
                let on = i as usize == self.active;
                let _ = li.set_attribute("aria-selected", if on { "true" } else { "false" });
                if on {
                    li.scroll_into_view_with_bool(false);
                }
            }
        }
        if self.results.is_empty() {
            let _ = self.input.remove_attribute("aria-activedescendant");
        } else {
            let _ = self.input.set_attribute(
                "aria-activedescendant",
                &format!("search-opt-{}", self.active),
            );
        }
    }

    fn move_active(&mut self, delta: isize) {
        let n = self.results.len() as isize;
        if n == 0 {
            return;
        }
        self.active = (self.active as isize + delta).rem_euclid(n) as usize;
        self.mark_active();
    }
}

fn ensure_loaded(st: &Shared) {
    {
        let mut s = st.borrow_mut();
        if !matches!(s.load, Load::Idle | Load::Failed) {
            return;
        }
        s.load = Load::Loading;
    }
    let st = st.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let loaded = async {
            let bytes = crate::data::fetch("places.tsv").await?;
            let text = String::from_utf8(bytes).map_err(|e| JsValue::from_str(&e.to_string()))?;
            Gazetteer::parse(&text).map_err(|e| JsValue::from_str(&e))
        }
        .await;
        let mut s = st.borrow_mut();
        match loaded {
            Ok(g) => {
                s.gaz = Some(g);
                s.load = Load::Done;
            }
            Err(e) => {
                web_sys::console::error_1(&e);
                s.load = Load::Failed;
            }
        }
        // Show results for whatever was typed while the file loaded.
        s.refresh();
    });
}

/// Fly to result `i`, then leave the box so the map has the keyboard again.
fn choose(st: &Shared, i: usize) {
    let (place, cb, input, doc) = {
        let mut s = st.borrow_mut();
        let Some(place) = s.results.get(i).cloned() else {
            return;
        };
        s.input.set_value(&place.name);
        s.hide_list();
        (place, s.on_select.clone(), s.input.clone(), s.doc.clone())
    };
    let _ = input.blur();
    if let Some(globe) = doc
        .get_element_by_id("globe")
        .and_then(|e| e.dyn_into::<HtmlElement>().ok())
    {
        let _ = globe.focus();
    }
    cb(
        place.lat.to_radians(),
        place.lon.to_radians(),
        place.fly_altitude(),
    );
}

/// Clear, close and give focus back to the map.
fn dismiss(st: &Shared) {
    let (input, doc) = {
        let mut s = st.borrow_mut();
        s.input.set_value("");
        s.hide_list();
        (s.input.clone(), s.doc.clone())
    };
    let _ = input.blur();
    if let Some(globe) = doc
        .get_element_by_id("globe")
        .and_then(|e| e.dyn_into::<HtmlElement>().ok())
    {
        let _ = globe.focus();
    }
}

/// Wire up the search box. `on_select(lat, lon, alt)` receives radians and
/// a camera altitude in globe radii.
pub fn init(doc: &Document, on_select: impl Fn(f64, f64, f64) + 'static) -> Result<(), JsValue> {
    let get = |id: &str| -> Result<Element, JsValue> {
        doc.get_element_by_id(id)
            .ok_or_else(|| JsValue::from_str(&format!("missing #{id}")))
    };
    let input: HtmlInputElement = get("search-input")?.dyn_into()?;
    let list: HtmlElement = get("search-results")?.dyn_into()?;
    let status: HtmlElement = get("search-status")?.dyn_into()?;
    let st: Shared = Rc::new(RefCell::new(State {
        doc: doc.clone(),
        input: input.clone(),
        list: list.clone(),
        status,
        gaz: None,
        load: Load::Idle,
        results: Vec::new(),
        active: 0,
        on_select: Rc::new(on_select),
    }));

    let input_target: &EventTarget = input.as_ref();
    {
        let st = st.clone();
        crate::listen(input_target, "focus", true, move |_: web_sys::Event| {
            ensure_loaded(&st);
            st.borrow_mut().refresh();
        })?;
    }
    {
        let st = st.clone();
        crate::listen(input_target, "input", true, move |_: web_sys::Event| {
            ensure_loaded(&st);
            st.borrow_mut().refresh();
        })?;
    }
    {
        let st = st.clone();
        crate::listen(input_target, "blur", true, move |_: web_sys::Event| {
            st.borrow_mut().hide_list();
        })?;
    }
    {
        let st = st.clone();
        crate::listen(input_target, "keydown", false, move |e: KeyboardEvent| {
            match e.key().as_str() {
                "ArrowDown" => st.borrow_mut().move_active(1),
                "ArrowUp" => st.borrow_mut().move_active(-1),
                "Enter" => {
                    let i = st.borrow().active;
                    choose(&st, i);
                }
                "Escape" => dismiss(&st),
                _ => return,
            }
            e.prevent_default();
            e.stop_propagation();
        })?;
    }
    // Keep focus in the input while a result is pressed, so the list is not
    // closed by the blur before the click lands.
    let list_target: &EventTarget = list.as_ref();
    {
        crate::listen(
            list_target,
            "pointerdown",
            false,
            move |e: web_sys::Event| {
                e.prevent_default();
            },
        )?;
    }
    {
        let st = st.clone();
        crate::listen(list_target, "click", true, move |e: web_sys::MouseEvent| {
            let index = e
                .target()
                .and_then(|t| t.dyn_into::<Element>().ok())
                .and_then(|el| el.closest("li").ok().flatten())
                .and_then(|li| li.get_attribute("data-index"))
                .and_then(|s| s.parse::<usize>().ok());
            if let Some(i) = index {
                choose(&st, i);
            }
        })?;
    }
    // `/` or Ctrl/Cmd+K focuses the box from anywhere outside a text field.
    {
        let input = input.clone();
        let window = web_sys::window().ok_or("no window")?;
        let window: &EventTarget = window.as_ref();
        crate::listen(window, "keydown", false, move |e: KeyboardEvent| {
            let key = e.key();
            let chord =
                (e.ctrl_key() || e.meta_key()) && !e.alt_key() && key.eq_ignore_ascii_case("k");
            let slash = key == "/" && !(e.ctrl_key() || e.meta_key() || e.alt_key());
            if (chord || slash) && !is_typing_target(&e) {
                e.prevent_default();
                let _ = input.focus();
                input.select();
            }
        })?;
    }
    Ok(())
}
