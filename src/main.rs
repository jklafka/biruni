//! Maps — an interactive globe written in Rust and compiled to WebAssembly.

mod camera;
mod controls;
mod country;
mod geo;
mod info_panel;
mod label_layer;
mod labels;
mod layer_loader;
mod lines;
mod measure;
mod overlay;
mod quality;
mod render;
mod search;
mod search_ui;
mod share;
mod sun;
mod view;

use camera::Camera;
use controls::{Controls, Direction, Event, Response};
use layer_loader::{Effect, LayerLoader};
use measure::Measure;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use view::{
    format_altitude, format_lat_lon, format_view_hash, parse_hash_layer, parse_view_hash,
    scale_bar, share_url, Layer, RedrawFlags, SunView, Units, HOME,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    AddEventListenerOptions, Document, EventTarget, HtmlCanvasElement, HtmlElement, KeyboardEvent,
    MouseEvent, PointerEvent, WheelEvent, Window,
};

type Action = fn(&mut App);
type FrameLoop = Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>>;

/// Longest the scale bar may be, in CSS pixels.
const SCALE_MAX_PX: f64 = 100.0;
/// How often the terminator follows the clock, in milliseconds.
const SUN_REFRESH_MS: i32 = 60_000;

struct App {
    window: Window,
    canvas: HtmlCanvasElement,
    renderer: render::Renderer,
    /// Which layer is shown, wanted and loading. See `layer_loader`.
    layers_state: LayerLoader<Layer>,
    /// Fetched data of layers that need it (none do yet).
    layer_data: HashMap<Layer, Rc<Vec<u8>>>,
    /// Handle to this app, for spawning layer fetches.
    me: Weak<RefCell<App>>,
    layer_status: HtmlElement,
    /// Set when the last load failed; cleared by the next selection.
    layer_error: Option<String>,
    layer_buttons: Vec<(Layer, HtmlElement)>,
    sun_on: bool,
    /// Fixed time from `&t=`; `None` follows the clock.
    sun_pin: Option<f64>,
    sun_button: HtmlElement,
    camera: Camera,
    /// Drag, pinch, inertia, zoom and flight state; mutates `camera`.
    controls: Controls,
    last_frame_ms: Option<f64>,
    /// Dynamic resolution policy and the scale of the last drawn frame.
    quality: quality::Quality,
    drawn_scale: f64,
    cursor: Option<(f64, f64)>,
    /// The last pointer was a mouse, so the cursor position means hover.
    mouse_hover: bool,
    countries: Option<country::Countries>,
    /// Country-ID raster, kept so a restored context can re-upload it.
    ids: Option<Rc<Vec<u8>>>,
    /// Country under the mouse cursor, and the one shown in the info panel.
    hover: Option<usize>,
    selected: Option<usize>,
    info: info_panel::InfoPanel,
    hud_country: HtmlElement,
    redraw: RedrawFlags,
    /// The URL hash needs to be rewritten once the view settles.
    hash_stale: bool,
    last_hash: String,
    hud_cursor: HtmlElement,
    hud_center: HtmlElement,
    hud_alt: HtmlElement,
    compass: HtmlElement,
    compass_needle: HtmlElement,
    hud_scale: HtmlElement,
    hud_scale_bar: HtmlElement,
    hud_scale_label: HtmlElement,
    units: Units,
    /// Decoded map data, kept so a restored context can re-upload it
    /// without refetching.
    layers: Option<Rc<geo::Layers>>,
    /// The `#loading` panel, reused for context-loss status.
    status: HtmlElement,
    share_ui: Rc<share::ShareUi>,
    measure: Measure,
    labels: label_layer::LabelLayer,
    keepout: label_layer::KeepOut,
    overlay: overlay::Overlay,
    measure_panel: overlay::Panel,
}

impl App {
    fn context_lost(&mut self) {
        self.renderer.context_lost();
        self.send(Event::CancelInertia);
        self.status.set_text_content(Some("Restoring map…"));
        let _ = self.status.class_list().remove_2("hidden", "error");
    }

    fn context_restored(&mut self) {
        let layers = self.layers.clone();
        match self
            .renderer
            .context_restored(layers.as_deref(), self.ids.as_ref().map(|v| v.as_slice()))
        {
            Ok(()) => {
                if layers.is_some() {
                    let _ = self.status.class_list().add_1("hidden");
                } else {
                    // Still downloading; the loader hides the panel when done.
                    self.status.set_text_content(Some("Loading map data…"));
                }
                self.redraw.mark_render();
            }
            Err(e) => {
                web_sys::console::error_1(&e);
                self.status
                    .set_text_content(Some("Could not restore the map. Reload the page."));
                let _ = self.status.class_list().add_1("error");
            }
        }
    }

    /// Store freshly loaded map data and upload it (deferred while lost).
    fn map_loaded(&mut self, layers: geo::Layers) -> Result<(), JsValue> {
        let layers = Rc::new(layers);
        self.layers = Some(layers.clone());
        self.renderer.upload_map(&layers)?;
        if !self.renderer.is_lost() {
            let _ = self.status.class_list().add_1("hidden");
        }
        self.redraw.mark_render();
        Ok(())
    }
}

impl App {
    fn now(&self) -> f64 {
        self.window.performance().map(|p| p.now()).unwrap_or(0.0)
    }

    /// Feed an input event to the controls; redraws if the view changed.
    fn send(&mut self, ev: Event) -> Response {
        let r = self.controls.handle(&mut self.camera, ev);
        if r.render {
            self.redraw.mark_render();
        }
        r
    }

    /// The user picked a layer (button, key, URL). Layers with data fetch it
    /// first and the globe keeps the previous layer until it is ready.
    fn select_layer(&mut self, layer: Layer) {
        let effects = self.layers_state.select(layer);
        self.apply_layer_effects(effects);
    }

    fn apply_layer_effects(&mut self, effects: Vec<Effect<Layer>>) {
        for effect in effects {
            match effect {
                Effect::Show(l) => {
                    // The camera is untouched.
                    debug_assert!(self.layers_state.shown() == l);
                    self.renderer.set_layer(l);
                    self.redraw.mark_render();
                }
                Effect::Fetch(l) => self.fetch_layer(l),
                Effect::Failed(l, msg) => {
                    web_sys::console::error_1(&JsValue::from_str(&msg));
                    self.layer_error =
                        Some(format!("Could not load {}. Select it to retry.", l.id()));
                }
            }
        }
        if self.layers_state.pending().is_some() {
            self.layer_error = None;
        }
        self.refresh_layer_ui();
        self.hash_stale = true;
    }

    fn fetch_layer(&mut self, layer: Layer) {
        let Some(app) = self.me.upgrade() else { return };
        let Some(url) = layer.data_url() else {
            // Cannot happen: only layers with data are fetched.
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            let result = geo::fetch_bytes(url)
                .await
                .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")));
            let mut a = app.borrow_mut();
            let result = result.map(|bytes| {
                a.layer_data.insert(layer, Rc::new(bytes));
            });
            let effects = a.layers_state.finished(layer, result);
            a.apply_layer_effects(effects);
        });
    }

    /// Picker buttons show the wanted layer as pressed and the one being
    /// fetched as busy; the status line carries load errors.
    fn refresh_layer_ui(&self) {
        let (wanted, pending) = (self.layers_state.wanted(), self.layers_state.pending());
        for (l, b) in &self.layer_buttons {
            let _ = b.set_attribute("aria-pressed", if *l == wanted { "true" } else { "false" });
            let _ = b.set_attribute(
                "aria-busy",
                if Some(*l) == pending { "true" } else { "false" },
            );
        }
        let text = match (pending, &self.layer_error) {
            (Some(l), _) => format!("Loading {}…", l.id()),
            (None, Some(e)) => e.clone(),
            _ => String::new(),
        };
        self.layer_status.set_text_content(Some(&text));
        self.layer_status.set_hidden(text.is_empty());
    }

    /// Push the current sun direction (or none when off) to the renderer
    /// and schedule a redraw.
    fn refresh_sun(&mut self) {
        let dir = if self.sun_on {
            let t = self.sun_pin.unwrap_or_else(|| js_sys::Date::now() / 1000.0);
            sun::sun_direction(t)
        } else {
            None
        };
        self.renderer.set_sun(dir);
        let _ = self
            .sun_button
            .set_attribute("aria-pressed", if self.sun_on { "true" } else { "false" });
        self.redraw.mark_render();
    }

    fn toggle_sun(&mut self) {
        self.sun_on = !self.sun_on;
        self.refresh_sun();
        self.hash_stale = true;
    }

    fn sun_view(&self) -> SunView {
        SunView {
            on: self.sun_on,
            pin: self.sun_pin,
        }
    }

    /// Link to the live view (not `location.hash`, which is debounced).
    fn current_share_url(&self) -> String {
        let page = self.window.location().href().unwrap_or_default();
        share_url(
            &page,
            &self.camera,
            self.layers_state.wanted(),
            self.sun_view(),
        )
    }

    /// Enter or leave measure mode. Leaving clears the path.
    fn toggle_measure(&mut self) {
        if self.measure.active {
            self.measure.exit();
        } else {
            self.measure.active = true;
            self.select(None); // the info panel would overlap the measure panel
        }
        self.measure_changed();
        self.update_hover();
    }

    fn measure_changed(&mut self) {
        let _ = if self.measure.active {
            self.canvas.class_list().add_1("measuring")
        } else {
            self.canvas.class_list().remove_1("measuring")
        };
        self.redraw.mark_render();
    }

    /// Place a measurement point under a screen position, if it hits the globe.
    fn measure_add_at(&mut self, sx: f64, sy: f64) {
        if let Some((lat, lon)) = self.camera.pick(sx, sy) {
            if self.measure.add(lat, lon) {
                self.measure_changed();
            }
        }
    }

    fn measure_undo(&mut self) {
        if self.measure.active && self.measure.undo() {
            self.measure_changed();
        }
    }

    /// Whether a plain click or tap on the globe opens the country panel.
    /// This is the one place to return false while another tool owns taps
    /// on the globe (measure mode places points instead).
    fn info_click_enabled(&self) -> bool {
        !self.measure.active
    }

    /// Country at a screen point, once the country data has loaded.
    fn country_at(&self, x: f64, y: f64) -> Option<usize> {
        let (lat, lon) = self.camera.pick(x, y)?;
        self.countries
            .as_ref()?
            .hit(lat.to_degrees(), lon.to_degrees())
    }

    /// Highlight follows the hovered country, else the selected one.
    fn sync_highlight(&mut self) {
        let id = self.hover.or(self.selected).map_or(0, |i| i as u8 + 1);
        if self.renderer.set_highlight(id) {
            self.redraw.mark_render();
        }
    }

    /// Re-test the country under the mouse. Only a change redraws.
    fn update_hover(&mut self) {
        // Hover highlighting is off while measuring: the cursor is placing
        // points there, and a tinted country would just be visual noise.
        let hit = match self.cursor {
            Some((x, y)) if self.mouse_hover && !self.measure.active => self.country_at(x, y),
            _ => None,
        };
        if hit != self.hover {
            self.hover = hit;
            self.sync_highlight();
            self.redraw.mark_hud();
        }
    }

    /// Open the info panel for a country, or close it with `None`.
    fn select(&mut self, country: Option<usize>) {
        self.selected = country;
        let c = country.and_then(|i| self.countries.as_ref()?.get(i));
        self.info.show(c);
        self.sync_highlight();
    }

    fn click(&mut self, x: f64, y: f64) {
        if self.info_click_enabled() {
            let hit = self.country_at(x, y);
            self.select(hit);
        }
    }

    fn frame(&mut self, now_ms: f64) {
        let dt = self
            .last_frame_ms
            .map(|t| (now_ms - t).clamp(0.0, 100.0))
            .unwrap_or(16.0);
        self.last_frame_ms = Some(now_ms);

        // Track canvas size (CSS and device pixels).
        let dpr = self.window.device_pixel_ratio().min(2.0);
        let (cw, ch) = (
            self.canvas.client_width() as f64,
            self.canvas.client_height() as f64,
        );
        let (pw, ph) = ((cw * dpr).round() as u32, (ch * dpr).round() as u32);
        if pw != self.canvas.width() || ph != self.canvas.height() {
            self.canvas.set_width(pw);
            self.canvas.set_height(ph);
            self.redraw.mark_render();
            self.keepout.invalidate(); // panels may have moved with the window
        }
        self.camera.width = cw;
        self.camera.height = ch;

        let animating = self.send(Event::Tick { dt }).render;
        if animating {
            self.update_hover(); // the globe moved under a still cursor
        }
        // Dynamic resolution: draw at a lower scale while the view moves, and
        // once more at full resolution after it has settled.
        let moving = self.controls.in_motion(&self.camera);
        let scale = self
            .quality
            .scale(moving, now_ms, dt, pw as f64 * ph as f64);
        if scale != self.drawn_scale {
            self.redraw.mark_render();
        }
        let (draw, hud) = self.redraw.take();
        let relayout = self
            .keepout
            .take(cw, ch)
            .is_some_and(|r| self.labels.set_blocked(r));
        if draw || relayout || self.labels.animating() {
            self.labels.update(&self.camera, dt, pw, ph, dpr);
        }
        if draw {
            self.renderer.draw(&self.camera, pw, ph, dpr, scale);
            self.drawn_scale = scale;
            self.overlay.draw(&self.camera, &self.measure, pw, ph, dpr);
            self.hash_stale = true;
        }
        if hud {
            self.update_hud();
        }
        if self.hash_stale && !animating && !self.controls.dragging() {
            self.hash_stale = false;
            let hash = format_view_hash(&self.camera, self.layers_state.wanted(), self.sun_view());
            if hash != self.last_hash {
                if let Ok(history) = self.window.history() {
                    let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&hash));
                }
                self.last_hash = hash;
            }
        }
    }

    fn update_hud(&mut self) {
        self.measure_panel.update(&self.measure, self.units);
        let cursor = self
            .cursor
            .and_then(|(x, y)| self.camera.pick(x, y))
            .map(|(lat, lon)| format_lat_lon(lat, lon))
            .unwrap_or_else(|| "—".into());
        self.hud_cursor.set_text_content(Some(&cursor));
        self.hud_center
            .set_text_content(Some(&format_lat_lon(self.camera.lat, self.camera.lon)));
        self.hud_alt
            .set_text_content(Some(&format_altitude(self.camera.alt)));
        // The needle points at north: the globe turned by the bearing, so
        // north sits that many degrees counterclockwise from screen up.
        let bearing = self.camera.bearing_deg();
        let _ = self
            .compass_needle
            .style()
            .set_property("transform", &format!("rotate({:.1}deg)", -bearing));
        let _ = self.compass.set_attribute(
            "aria-pressed",
            if self.camera.north_up {
                "true"
            } else {
                "false"
            },
        );
        let _ = self.compass.set_attribute(
            "title",
            &format!(
                "Bearing {:.0}°. Click to face north and lock north-up (F toggles the lock)",
                bearing
            ),
        );
        let country = self
            .hover
            .and_then(|i| self.countries.as_ref()?.get(i))
            .map_or("—", |c| c.name.as_str());
        self.hud_country.set_text_content(Some(country));
        match scale_bar(&self.camera, self.units, SCALE_MAX_PX) {
            Some(bar) => {
                let _ = self.hud_scale.style().set_property("display", "");
                let _ = self
                    .hud_scale_bar
                    .style()
                    .set_property("width", &format!("{:.0}px", bar.px));
                self.hud_scale_label.set_text_content(Some(&bar.label));
            }
            None => {
                let _ = self.hud_scale.style().set_property("display", "none");
            }
        }
    }

    // ---- pointer input -------------------------------------------------

    fn pointer_down(&mut self, e: &PointerEvent) {
        if e.pointer_type() == "mouse" && e.button() != 0 {
            return;
        }
        let _ = self.canvas.set_pointer_capture(e.pointer_id());
        let t = self.now();
        self.send(Event::PointerDown {
            id: e.pointer_id(),
            x: e.client_x() as f64,
            y: e.client_y() as f64,
            t,
        });
        let _ = self.canvas.class_list().add_1("dragging");
    }

    fn pointer_move(&mut self, e: &PointerEvent) {
        let (x, y) = (e.client_x() as f64, e.client_y() as f64);
        self.cursor = Some((x, y));
        self.mouse_hover = e.pointer_type() == "mouse";
        // The cursor readout alone never needs a redraw of the globe.
        self.redraw.mark_hud();
        self.update_hover();
        let t = self.now();
        self.send(Event::PointerMove {
            id: e.pointer_id(),
            x,
            y,
            t,
            alt: e.alt_key(),
        });
    }

    fn pointer_up(&mut self, e: &PointerEvent) {
        let t = self.now();
        let r = self.send(Event::PointerUp {
            id: e.pointer_id(),
            t,
            cancel: e.type_() != "pointerup",
        });
        // The single tap detector: a lone press released near where it went
        // down. Measure mode places a point; otherwise it selects a country.
        if let Some((x, y)) = r.tap {
            if self.measure.active {
                self.measure_add_at(x, y);
            } else {
                self.click(x, y);
            }
        }
        if !self.controls.dragging() {
            let _ = self.canvas.class_list().remove_1("dragging");
        }
    }
}

fn element<T: JsCast>(doc: &Document, id: &str) -> Result<T, JsValue> {
    doc.get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("missing #{id}")))?
        .dyn_into::<T>()
        .map_err(|_| JsValue::from_str(&format!("#{id} has the wrong element type")))
}

fn listen<E: JsCast + 'static>(
    target: &EventTarget,
    event: &str,
    passive: bool,
    mut handler: impl FnMut(E) + 'static,
) -> Result<(), JsValue> {
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
        handler(e.unchecked_into::<E>())
    });
    let opts = AddEventListenerOptions::new();
    opts.set_passive(passive);
    target.add_event_listener_with_callback_and_add_event_listener_options(
        event,
        closure.as_ref().unchecked_ref(),
        &opts,
    )?;
    closure.forget(); // listeners live for the whole page
    Ok(())
}

fn start() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let doc = window.document().ok_or("no document")?;
    let canvas: HtmlCanvasElement = element(&doc, "globe")?;
    let loading: HtmlElement = element(&doc, "loading")?;

    let renderer = match render::Renderer::new(&canvas) {
        Ok(r) => r,
        Err(e) => {
            loading.set_text_content(Some(&format!(
                "Could not start WebGL2: {}",
                e.as_string().unwrap_or_default()
            )));
            let _ = loading.class_list().add_1("error");
            return Err(e);
        }
    };

    let camera = window
        .location()
        .hash()
        .ok()
        .and_then(|h| parse_view_hash(&h))
        .unwrap_or_else(|| Camera::new(HOME.0, HOME.1, HOME.2));

    let initial_layer = window
        .location()
        .hash()
        .ok()
        .and_then(|h| parse_hash_layer(&h))
        .unwrap_or_default();
    let sun_hash = window
        .location()
        .hash()
        .map(|h| SunView::from_hash(&h))
        .unwrap_or_default();
    let mut layer_buttons = Vec::new();
    for layer in Layer::ALL {
        let b: HtmlElement = element(&doc, &format!("layer-{}", layer.id()))?;
        layer_buttons.push((layer, b));
    }

    let overlay_canvas: HtmlCanvasElement = element(&doc, "overlay")?;
    let overlay = overlay::Overlay::new(overlay_canvas)?;
    let labels = label_layer::LabelLayer::new(element(&doc, "labels")?)?;
    let measure_panel = overlay::Panel::new(&doc)?;

    let app = Rc::new(RefCell::new(App {
        window: window.clone(),
        canvas: canvas.clone(),
        renderer,
        layers_state: LayerLoader::new(Layer::default()),
        layer_data: HashMap::new(),
        me: Weak::new(),
        layer_status: element(&doc, "layer-status")?,
        layer_error: None,
        layer_buttons: layer_buttons.clone(),
        sun_on: sun_hash.on,
        sun_pin: sun_hash.pin,
        sun_button: element(&doc, "sun-toggle")?,
        camera,
        controls: Controls::new(&camera),
        last_frame_ms: None,
        quality: quality::Quality::new(quality::parse_setting(
            &window.location().search().unwrap_or_default(),
        )),
        drawn_scale: 1.0,
        cursor: None,
        mouse_hover: false,
        countries: None,
        ids: None,
        hover: None,
        selected: None,
        info: info_panel::InfoPanel::new(&doc)?,
        hud_country: element(&doc, "hud-country")?,
        redraw: RedrawFlags::new(),
        hash_stale: false,
        last_hash: String::new(),
        hud_cursor: element(&doc, "hud-cursor")?,
        hud_center: element(&doc, "hud-center")?,
        hud_alt: element(&doc, "hud-alt")?,
        compass: element(&doc, "compass")?,
        compass_needle: element(&doc, "compass-needle")?,
        hud_scale: element(&doc, "hud-scale")?,
        hud_scale_bar: element(&doc, "hud-scale-bar")?,
        hud_scale_label: element(&doc, "hud-scale-label")?,
        units: Units::Metric,
        layers: None,
        status: loading.clone(),
        share_ui: share::ShareUi::new(&window, &doc, canvas.clone().into())?,
        measure: Measure::default(),
        labels,
        keepout: label_layer::KeepOut::new(&doc)?,
        overlay,
        measure_panel,
    }));

    {
        let mut a = app.borrow_mut();
        a.me = Rc::downgrade(&app);
        // The URL layer takes the same path as a click.
        a.select_layer(initial_layer);
    }
    app.borrow_mut().refresh_sun();

    // The one piece of idle work: once a minute, move the terminator. It
    // only marks a render while the terminator is on and follows the clock.
    {
        let app = app.clone();
        let tick = Closure::<dyn FnMut()>::new(move || {
            let mut a = app.borrow_mut();
            if a.sun_on && a.sun_pin.is_none() {
                a.refresh_sun();
            }
        });
        window.set_interval_with_callback_and_timeout_and_arguments_0(
            tick.as_ref().unchecked_ref(),
            SUN_REFRESH_MS,
        )?;
        tick.forget();
    }
    {
        let button: HtmlElement = element(&doc, "sun-toggle")?;
        let app = app.clone();
        listen(button.as_ref(), "click", true, move |_: MouseEvent| {
            app.borrow_mut().toggle_sun();
        })?;
    }
    for (layer, button) in layer_buttons {
        let app = app.clone();
        listen(button.as_ref(), "click", true, move |_: MouseEvent| {
            app.borrow_mut().select_layer(layer);
        })?;
    }

    // Pointer input (mouse, touch, pen).
    let target: &EventTarget = canvas.as_ref();
    {
        let app = app.clone();
        listen(target, "pointerdown", true, move |e: PointerEvent| {
            app.borrow_mut().pointer_down(&e)
        })?;
    }
    {
        let app = app.clone();
        listen(target, "pointermove", true, move |e: PointerEvent| {
            app.borrow_mut().pointer_move(&e)
        })?;
    }
    for name in ["pointerup", "pointercancel"] {
        let app = app.clone();
        listen(target, name, true, move |e: PointerEvent| {
            app.borrow_mut().pointer_up(&e)
        })?;
    }
    {
        let app = app.clone();
        listen(target, "pointerleave", true, move |e: PointerEvent| {
            if e.pointer_type() == "mouse" {
                let mut a = app.borrow_mut();
                a.cursor = None;
                a.update_hover();
                a.redraw.mark_hud();
            }
        })?;
    }
    {
        let app = app.clone();
        listen(target, "wheel", false, move |e: WheelEvent| {
            e.prevent_default();
            app.borrow_mut().send(Event::Wheel {
                dy: e.delta_y(),
                x: e.client_x() as f64,
                y: e.client_y() as f64,
                mode: e.delta_mode(),
                ctrl: e.ctrl_key(),
            });
        })?;
    }
    {
        let app = app.clone();
        listen(target, "dblclick", true, move |e: MouseEvent| {
            let mut a = app.borrow_mut();
            if a.measure.active {
                return; // the taps already placed points
            }
            a.send(Event::DoubleClick {
                x: e.client_x() as f64,
                y: e.client_y() as f64,
                shift: e.shift_key(),
            });
        })?;
    }

    // Clicking or tapping the scale bar toggles metric/imperial.
    {
        let app = app.clone();
        let scale: EventTarget = app.borrow().hud_scale.clone().into();
        listen(&scale, "click", true, move |_: MouseEvent| {
            let mut a = app.borrow_mut();
            a.units = a.units.toggled();
            a.redraw.mark_hud();
        })?;
    }

    // WebGL context loss: preventDefault is required for restoration.
    {
        let app = app.clone();
        listen(
            target,
            "webglcontextlost",
            false,
            move |e: web_sys::Event| {
                e.prevent_default();
                app.borrow_mut().context_lost();
            },
        )?;
    }
    {
        let app = app.clone();
        listen(
            target,
            "webglcontextrestored",
            true,
            move |_: web_sys::Event| app.borrow_mut().context_restored(),
        )?;
    }

    // Keyboard.
    {
        let app = app.clone();
        listen(window.as_ref(), "keydown", true, move |e: KeyboardEvent| {
            // Typing in a text field (the search box) is not a map shortcut.
            if e.target().is_some_and(|t| {
                t.is_instance_of::<web_sys::HtmlInputElement>()
                    || t.is_instance_of::<web_sys::HtmlTextAreaElement>()
            }) {
                return;
            }
            let mut a = app.borrow_mut();
            match e.key().as_str() {
                "ArrowLeft" => {
                    a.send(Event::Nudge(Direction::Left));
                }
                "ArrowRight" => {
                    a.send(Event::Nudge(Direction::Right));
                }
                "ArrowUp" => {
                    a.send(Event::Nudge(Direction::Up));
                }
                "ArrowDown" => {
                    a.send(Event::Nudge(Direction::Down));
                }
                "+" | "=" => {
                    a.send(Event::Zoom {
                        factor: 0.6,
                        toward: None,
                    });
                }
                "-" | "_" => {
                    a.send(Event::Zoom {
                        factor: 1.0 / 0.6,
                        toward: None,
                    });
                }
                "r" | "R" => {
                    a.send(Event::Reset);
                }
                "f" | "F" if !(e.ctrl_key() || e.meta_key() || e.alt_key()) => {
                    a.send(Event::ToggleNorthUp);
                }
                "q" | "Q" if !(e.ctrl_key() || e.meta_key() || e.alt_key()) => {
                    a.send(Event::Rotate(-controls::ROTATE_STEP));
                }
                "e" | "E" if !(e.ctrl_key() || e.meta_key() || e.alt_key()) => {
                    a.send(Event::Rotate(controls::ROTATE_STEP));
                }
                "i" | "I" if a.info_click_enabled() => {
                    // Info for the country at the center of the view.
                    let (lat, lon) = (a.camera.lat.to_degrees(), a.camera.lon.to_degrees());
                    let hit = a.countries.as_ref().and_then(|c| c.hit(lat, lon));
                    a.select(hit);
                    return;
                }
                "u" | "U" => {
                    // Only the HUD text changes; no need to redraw the globe.
                    a.units = a.units.toggled();
                    a.redraw.mark_hud();
                    return;
                }
                "s" | "S" if !(e.ctrl_key() || e.meta_key() || e.alt_key()) => {
                    // Sharing never changes the view, so no redraw.
                    let (ui, url) = (a.share_ui.clone(), a.current_share_url());
                    drop(a);
                    share::share(&ui, url);
                    return;
                }
                "n" | "N" if !(e.ctrl_key() || e.meta_key() || e.alt_key()) => a.toggle_sun(),
                "m" | "M" => a.toggle_measure(),
                "Escape" if a.measure.active => a.toggle_measure(),
                "Escape" => {
                    // Not measuring: close the info panel.
                    a.select(None);
                    return;
                }
                "Backspace" if a.measure.active => a.measure_undo(),
                "Enter" | " " if a.measure.active => {
                    // Keyboard path: drop a point at the view center. Skip when
                    // a button has focus, where Enter/Space activates it.
                    let on_canvas = e.target().is_some_and(|t| {
                        let c: &EventTarget = a.canvas.as_ref();
                        &t == c
                    });
                    if !on_canvas {
                        return;
                    }
                    let (cx, cy) = (a.camera.width * 0.5, a.camera.height * 0.5);
                    a.measure_add_at(cx, cy);
                }
                "l" | "L" => {
                    let next = a.layers_state.wanted().next();
                    a.select_layer(next);
                }
                k @ ("1" | "2") => {
                    let i = k.parse::<usize>().unwrap_or(1) - 1;
                    if let Some(l) = Layer::ALL.get(i) {
                        a.select_layer(*l);
                    }
                }
                _ => return,
            }
            a.redraw.mark_render();
        })?;
    }

    // Editing the URL hash by hand flies to that view.
    {
        let app = app.clone();
        let window2 = window.clone();
        listen(
            window.as_ref(),
            "hashchange",
            true,
            move |_: web_sys::Event| {
                let hash = window2.location().hash().unwrap_or_default();
                let mut a = app.borrow_mut();
                a.select_layer(parse_hash_layer(&hash).unwrap_or_default());
                let sun = SunView::from_hash(&hash);
                a.sun_on = sun.on;
                a.sun_pin = sun.pin;
                a.refresh_sun();
                if let Some(c) = parse_view_hash(&hash) {
                    a.send(Event::FlyTo {
                        lat: c.lat,
                        lon: c.lon,
                        alt: c.alt,
                        bearing: Some(c.bearing),
                    });
                }
            },
        )?;
    }

    // On-screen buttons.
    let buttons: [(&str, Action); 8] = [
        ("compass", |a| {
            a.send(Event::NorthUp);
        }),
        ("measure-toggle", |a| a.toggle_measure()),
        ("measure-undo", |a| a.measure_undo()),
        ("measure-exit", |a| {
            if a.measure.active {
                a.toggle_measure()
            }
        }),
        ("zoom-in", |a| {
            a.send(Event::Zoom {
                factor: 0.5,
                toward: None,
            });
        }),
        ("zoom-out", |a| {
            a.send(Event::Zoom {
                factor: 2.0,
                toward: None,
            });
        }),
        ("reset", |a| {
            a.send(Event::Reset);
        }),
        ("ci-close", |a| a.select(None)),
    ];
    for (id, action) in buttons {
        let button: HtmlElement = element(&doc, id)?;
        let app = app.clone();
        listen(button.as_ref(), "click", true, move |_: MouseEvent| {
            action(&mut app.borrow_mut())
        })?;
    }

    // Place search: fly to the chosen place.
    {
        let app = app.clone();
        search_ui::init(&doc, move |lat, lon, alt| {
            app.borrow_mut().send(Event::FlyTo {
                lat,
                lon,
                alt,
                bearing: None,
            });
        })?;
    }

    // Share button.
    {
        let button: HtmlElement = element(&doc, "share")?;
        let app = app.clone();
        listen(button.as_ref(), "click", true, move |_: MouseEvent| {
            let (ui, url) = {
                let a = app.borrow();
                (a.share_ui.clone(), a.current_share_url())
            };
            share::share(&ui, url);
        })?;
    }

    // Animation loop: runs every frame, redraws only when something changed.
    let raf: FrameLoop = Rc::new(RefCell::new(None));
    {
        let raf_inner = raf.clone();
        let app = app.clone();
        let window = window.clone();
        *raf.borrow_mut() = Some(Closure::new(move |t: f64| {
            app.borrow_mut().frame(t);
            if let Some(cb) = raf_inner.borrow().as_ref() {
                let _ = window.request_animation_frame(cb.as_ref().unchecked_ref());
            }
        }));
    }
    window.request_animation_frame(raf.borrow().as_ref().unwrap().as_ref().unchecked_ref())?;

    // Load and rasterize the map data in the background.
    wasm_bindgen_futures::spawn_local(async move {
        let result = match geo::Layers::load().await {
            Ok(layers) => app.borrow_mut().map_loaded(layers),
            Err(e) => Err(e),
        };
        let ok = result.is_ok();
        match result {
            Ok(()) => {}
            Err(e) => {
                let msg = e.as_string().unwrap_or_else(|| format!("{e:?}"));
                web_sys::console::error_1(&e);
                loading.set_text_content(Some(&format!("Failed to load map data: {msg}")));
                let _ = loading.class_list().add_1("error");
            }
        }
        // Place labels are small (about 125 KB) and drawn on every view, so
        // they come right after the first frame. A failure just means no labels.
        if ok {
            match label_layer::load().await {
                Ok(list) => {
                    let mut a = app.borrow_mut();
                    a.labels.set_labels(list);
                    a.redraw.mark_render();
                }
                Err(e) => web_sys::console::error_1(&e),
            }
        }
        // Country polygons are only needed for hover and click, so they load
        // after the first frame. A failure here just disables those.
        if ok {
            match country::load().await {
                Ok(countries) => {
                    let mut a = app.borrow_mut();
                    let ids_width = a.renderer.ids_width();
                    let ids = countries.rasterize_ids(ids_width);
                    // No-op while the context is lost; context_restored uploads it.
                    if let Err(e) = a.renderer.upload_country_ids(&ids, ids_width) {
                        web_sys::console::error_1(&e);
                    }
                    a.ids = Some(Rc::new(ids));
                    a.countries = Some(countries);
                    a.update_hover();
                }
                Err(e) => web_sys::console::error_1(&e),
            }
        }
    });

    let _ = canvas.focus();
    Ok(())
}

fn main() {
    console_error_panic_hook::set_once();
    if let Err(e) = start() {
        web_sys::console::error_1(&e);
    }
}
