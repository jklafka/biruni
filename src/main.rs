//! Maps — an interactive globe written in Rust and compiled to WebAssembly.

mod camera;
mod country;
mod geo;
mod info_panel;
mod measure;
mod overlay;
mod render;
mod share;
mod view;

use camera::{wrap_angle, Camera, MAX_ALT, MIN_ALT};
use measure::Measure;
use std::cell::RefCell;
use std::rc::Rc;
use view::{
    format_altitude, format_lat_lon, format_view_hash, parse_hash_layer, parse_view_hash,
    scale_bar, share_url, Layer, RedrawFlags, Units, HOME,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    AddEventListenerOptions, Document, EventTarget, HtmlCanvasElement, HtmlElement, KeyboardEvent,
    MouseEvent, PointerEvent, WheelEvent, Window,
};

type Action = fn(&mut App);
type FrameLoop = Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>>;

struct Pointer {
    id: i32,
    x: f64,
    y: f64,
}

/// A lone pointer press that has not moved past `TAP_SLOP_PX` yet. Released
/// in place it is a tap: it places a measure point or selects a country.
struct Press {
    x: f64,
    y: f64,
}

/// A surface point pinned under a screen position while zooming.
struct Anchor {
    lat: f64,
    lon: f64,
    sx: f64,
    sy: f64,
}

/// Smooth flight of the view center to a target.
struct Flight {
    lat: f64,
    lon: f64,
}

/// Longest the scale bar may be, in CSS pixels.
const SCALE_MAX_PX: f64 = 100.0;
/// A press that moves less than this many CSS pixels is a tap, not a drag.
const TAP_SLOP_PX: f64 = 6.0;

struct App {
    window: Window,
    canvas: HtmlCanvasElement,
    renderer: render::Renderer,
    layer: Layer,
    layer_buttons: Vec<(Layer, HtmlElement)>,
    camera: Camera,
    target_alt: f64,
    anchor: Option<Anchor>,
    flight: Option<Flight>,
    pointers: Vec<Pointer>,
    pinch: Option<f64>,
    /// Pan inertia in CSS pixels per millisecond.
    velocity: (f64, f64),
    last_move_ms: f64,
    last_frame_ms: Option<f64>,
    cursor: Option<(f64, f64)>,
    /// A primary press that may still turn out to be a click (not a drag).
    press: Option<Press>,
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
    overlay: overlay::Overlay,
    measure_panel: overlay::Panel,
}

impl App {
    fn context_lost(&mut self) {
        self.renderer.context_lost();
        self.velocity = (0.0, 0.0);
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

    fn stop_motion(&mut self) {
        self.velocity = (0.0, 0.0);
        self.flight = None;
        self.anchor = None;
        self.target_alt = self.camera.alt;
    }

    /// Zoom by a multiplicative factor on altitude, toward a screen point.
    fn zoom(&mut self, factor: f64, toward: Option<(f64, f64)>) {
        self.flight = None;
        self.target_alt = (self.target_alt * factor).clamp(MIN_ALT, MAX_ALT);
        self.anchor = toward.and_then(|(sx, sy)| {
            self.camera
                .pick(sx, sy)
                .map(|(lat, lon)| Anchor { lat, lon, sx, sy })
        });
        self.redraw.mark_render();
    }

    fn fly_to(&mut self, lat: f64, lon: f64, alt: f64) {
        self.stop_motion();
        self.flight = Some(Flight { lat, lon });
        self.target_alt = alt.clamp(MIN_ALT, MAX_ALT);
        self.redraw.mark_render();
    }

    /// Switch the map style. The camera is untouched.
    fn set_layer(&mut self, layer: Layer) {
        self.layer = layer;
        self.renderer.set_layer(layer);
        for (l, b) in &self.layer_buttons {
            let _ = b.set_attribute("aria-pressed", if *l == layer { "true" } else { "false" });
        }
        self.redraw.mark_render();
        self.hash_stale = true;
    }

    /// Link to the live view (not `location.hash`, which is debounced).
    fn current_share_url(&self) -> String {
        let page = self.window.location().href().unwrap_or_default();
        share_url(&page, &self.camera, self.layer)
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

    fn reset(&mut self) {
        self.fly_to(HOME.0.to_radians(), HOME.1.to_radians(), HOME.2);
    }

    /// Advance animations; returns true if anything moved.
    fn step(&mut self, dt: f64) -> bool {
        let mut moved = false;
        let ease = 1.0 - (-dt / 85.0).exp(); // ~85 ms time constant

        if let Some(f) = &self.flight {
            let dlat = f.lat - self.camera.lat;
            let dlon = wrap_angle(f.lon - self.camera.lon);
            if dlat.abs() < 1e-5 && dlon.abs() < 1e-5 {
                self.flight = None;
            } else {
                // Slower than zoom so the flight reads as a single gesture.
                let k = 1.0 - (-dt / 160.0).exp();
                self.camera.lat += dlat * k;
                self.camera.lon = wrap_angle(self.camera.lon + dlon * k);
                moved = true;
            }
        }

        let (la, lt) = (self.camera.alt.ln(), self.target_alt.ln());
        if (lt - la).abs() > 1e-4 {
            self.camera.alt = (la + (lt - la) * ease).exp();
            if let Some(a) = &self.anchor {
                let (lat, lon, sx, sy) = (a.lat, a.lon, a.sx, a.sy);
                self.camera.keep_point_at(lat, lon, sx, sy);
            }
            moved = true;
        } else if self.camera.alt != self.target_alt {
            self.camera.alt = self.target_alt;
            self.anchor = None;
            moved = true;
        }

        if self.pointers.is_empty() {
            let (vx, vy) = self.velocity;
            if vx.hypot(vy) > 0.005 {
                self.camera.pan_pixels(vx * dt, vy * dt);
                let decay = (-dt / 325.0).exp();
                self.velocity = (vx * decay, vy * decay);
                moved = true;
            } else {
                self.velocity = (0.0, 0.0);
            }
        }
        moved
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
        }
        self.camera.width = cw;
        self.camera.height = ch;

        let animating = self.step(dt);
        if animating {
            self.update_hover(); // the globe moved under a still cursor
            self.redraw.mark_render();
        }
        let (draw, hud) = self.redraw.take();
        if draw {
            self.renderer.draw(&self.camera, pw, ph, dpr);
            self.overlay.draw(&self.camera, &self.measure, pw, ph, dpr);
            self.hash_stale = true;
        }
        if hud {
            self.update_hud();
        }
        if self.hash_stale && !animating && self.pointers.is_empty() {
            self.hash_stale = false;
            let hash = format_view_hash(&self.camera, self.layer);
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
        self.stop_motion();
        self.pointers.retain(|p| p.id != e.pointer_id());
        self.press = if self.pointers.is_empty() {
            Some(Press {
                x: e.client_x() as f64,
                y: e.client_y() as f64,
            })
        } else {
            None // a second finger makes this a pinch
        };
        self.pointers.push(Pointer {
            id: e.pointer_id(),
            x: e.client_x() as f64,
            y: e.client_y() as f64,
        });
        self.pinch = None;
        self.last_move_ms = self.now();
        let _ = self.canvas.class_list().add_1("dragging");
    }

    fn pointer_move(&mut self, e: &PointerEvent) {
        let (x, y) = (e.client_x() as f64, e.client_y() as f64);
        self.cursor = Some((x, y));
        self.mouse_hover = e.pointer_type() == "mouse";
        // The cursor readout alone never needs a redraw of the globe.
        self.redraw.mark_hud();
        if self
            .press
            .as_ref()
            .is_some_and(|p| (x - p.x).hypot(y - p.y) > TAP_SLOP_PX)
        {
            self.press = None;
        }
        self.update_hover();

        let Some(idx) = self.pointers.iter().position(|p| p.id == e.pointer_id()) else {
            return;
        };
        self.redraw.mark_render();
        let now = self.now();
        let (px, py) = (self.pointers[idx].x, self.pointers[idx].y);

        if self.pointers.len() == 1 {
            let (dx, dy) = (x - px, y - py);
            self.camera.pan_pixels(dx, dy);
            let dt = (now - self.last_move_ms).max(1.0);
            let (ix, iy) = (dx / dt, dy / dt);
            self.velocity = (
                0.75 * ix + 0.25 * self.velocity.0,
                0.75 * iy + 0.25 * self.velocity.1,
            );
        } else {
            // Pinch: zoom by finger distance, pan with the midpoint.
            let (a, b) = (&self.pointers[0], &self.pointers[1]);
            let old_mid = ((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
            let old_dist = (a.x - b.x).hypot(a.y - b.y);
            self.pointers[idx].x = x;
            self.pointers[idx].y = y;
            let (a, b) = (&self.pointers[0], &self.pointers[1]);
            let new_mid = ((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
            let new_dist = (a.x - b.x).hypot(a.y - b.y);
            let target = self.camera.pick(old_mid.0, old_mid.1);
            if old_dist > 1.0 && new_dist > 1.0 {
                self.camera.alt = (self.camera.alt * old_dist / new_dist).clamp(MIN_ALT, MAX_ALT);
                self.target_alt = self.camera.alt;
            }
            match target {
                Some((lat, lon)) => self.camera.keep_point_at(lat, lon, new_mid.0, new_mid.1),
                None => self
                    .camera
                    .pan_pixels(new_mid.0 - old_mid.0, new_mid.1 - old_mid.1),
            }
            self.pinch = Some(new_dist);
        }
        self.pointers[idx].x = x;
        self.pointers[idx].y = y;
        self.last_move_ms = now;
    }

    fn pointer_up(&mut self, e: &PointerEvent) {
        self.pointers.retain(|p| p.id != e.pointer_id());
        // The single tap detector: a lone press released near where it went
        // down. Measure mode places a point; otherwise it selects a country.
        if let Some(p) = self.press.take() {
            if self.pointers.is_empty() && e.type_() == "pointerup" {
                if self.measure.active {
                    self.measure_add_at(p.x, p.y);
                } else {
                    self.click(p.x, p.y);
                }
            }
        }
        if self.pinch.is_some() || self.now() - self.last_move_ms > 80.0 {
            self.velocity = (0.0, 0.0); // no fling after a pinch or a pause
        }
        if self.pointers.is_empty() {
            self.pinch = None;
            let _ = self.canvas.class_list().remove_1("dragging");
        }
        self.redraw.mark_render();
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
    let mut layer_buttons = Vec::new();
    for layer in Layer::ALL {
        let b: HtmlElement = element(&doc, &format!("layer-{}", layer.id()))?;
        layer_buttons.push((layer, b));
    }

    let overlay_canvas: HtmlCanvasElement = element(&doc, "overlay")?;
    let overlay = overlay::Overlay::new(overlay_canvas)?;
    let measure_panel = overlay::Panel::new(&doc)?;

    let app = Rc::new(RefCell::new(App {
        window: window.clone(),
        canvas: canvas.clone(),
        renderer,
        layer: initial_layer,
        layer_buttons: layer_buttons.clone(),
        camera,
        target_alt: camera.alt,
        anchor: None,
        flight: None,
        pointers: Vec::new(),
        pinch: None,
        velocity: (0.0, 0.0),
        last_move_ms: 0.0,
        last_frame_ms: None,
        cursor: None,
        press: None,
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
        hud_scale: element(&doc, "hud-scale")?,
        hud_scale_bar: element(&doc, "hud-scale-bar")?,
        hud_scale_label: element(&doc, "hud-scale-label")?,
        units: Units::Metric,
        layers: None,
        status: loading.clone(),
        share_ui: share::ShareUi::new(&window, &doc, canvas.clone().into())?,
        measure: Measure::default(),
        overlay,
        measure_panel,
    }));

    app.borrow_mut().set_layer(initial_layer);
    for (layer, button) in layer_buttons {
        let app = app.clone();
        listen(button.as_ref(), "click", true, move |_: MouseEvent| {
            app.borrow_mut().set_layer(layer);
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
            let scale = match e.delta_mode() {
                1 => 16.0,  // lines
                2 => 400.0, // pages
                _ => 1.0,
            };
            // Trackpad pinch arrives as ctrl+wheel with small deltas.
            let k = if e.ctrl_key() { 0.01 } else { 0.0018 };
            let factor = (e.delta_y() * scale * k).clamp(-1.0, 1.0).exp();
            app.borrow_mut()
                .zoom(factor, Some((e.client_x() as f64, e.client_y() as f64)));
        })?;
    }
    {
        let app = app.clone();
        listen(target, "dblclick", true, move |e: MouseEvent| {
            let mut a = app.borrow_mut();
            if a.measure.active {
                return; // the taps already placed points
            }
            if let Some((lat, lon)) = a.camera.pick(e.client_x() as f64, e.client_y() as f64) {
                let alt = a.camera.alt * if e.shift_key() { 2.5 } else { 0.4 };
                a.fly_to(lat, lon, alt);
            }
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
            let mut a = app.borrow_mut();
            let step = 0.6; // px/ms impulse, decays via inertia
            match e.key().as_str() {
                "ArrowLeft" => a.velocity.0 = step,
                "ArrowRight" => a.velocity.0 = -step,
                "ArrowUp" => a.velocity.1 = step,
                "ArrowDown" => a.velocity.1 = -step,
                "+" | "=" => a.zoom(0.6, None),
                "-" | "_" => a.zoom(1.0 / 0.6, None),
                "r" | "R" => a.reset(),
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
                    let next = a.layer.next();
                    a.set_layer(next);
                }
                k @ ("1" | "2") => {
                    let i = k.parse::<usize>().unwrap_or(1) - 1;
                    if let Some(l) = Layer::ALL.get(i) {
                        a.set_layer(*l);
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
                a.set_layer(parse_hash_layer(&hash).unwrap_or_default());
                if let Some(c) = parse_view_hash(&hash) {
                    a.fly_to(c.lat, c.lon, c.alt);
                }
            },
        )?;
    }

    // On-screen buttons.
    let buttons: [(&str, Action); 7] = [
        ("measure-toggle", |a| a.toggle_measure()),
        ("measure-undo", |a| a.measure_undo()),
        ("measure-exit", |a| {
            if a.measure.active {
                a.toggle_measure()
            }
        }),
        ("zoom-in", |a| a.zoom(0.5, None)),
        ("zoom-out", |a| a.zoom(2.0, None)),
        ("reset", |a| a.reset()),
        ("ci-close", |a| a.select(None)),
    ];
    for (id, action) in buttons {
        let button: HtmlElement = element(&doc, id)?;
        let app = app.clone();
        listen(button.as_ref(), "click", true, move |_: MouseEvent| {
            action(&mut app.borrow_mut())
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
        // Country polygons are only needed for hover and click, so they load
        // after the first frame. A failure here just disables those.
        if ok {
            match country::load().await {
                Ok(countries) => {
                    let ids = countries.rasterize_ids(country::ID_RASTER_WIDTH);
                    let mut a = app.borrow_mut();
                    // No-op while the context is lost; context_restored uploads it.
                    if let Err(e) = a
                        .renderer
                        .upload_country_ids(&ids, country::ID_RASTER_WIDTH)
                    {
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
