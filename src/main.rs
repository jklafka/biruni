//! Maps — an interactive globe written in Rust and compiled to WebAssembly.

mod camera;
mod geo;
mod render;
mod view;

use camera::{wrap_angle, Camera, MAX_ALT, MIN_ALT};
use std::cell::RefCell;
use std::rc::Rc;
use view::{format_altitude, format_lat_lon, format_view_hash, parse_view_hash, RedrawFlags, HOME};
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

struct App {
    window: Window,
    canvas: HtmlCanvasElement,
    renderer: render::Renderer,
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
    redraw: RedrawFlags,
    /// The URL hash needs to be rewritten once the view settles.
    hash_stale: bool,
    last_hash: String,
    hud_cursor: HtmlElement,
    hud_center: HtmlElement,
    hud_alt: HtmlElement,
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
            self.redraw.mark_render();
        }
        let (draw, hud) = self.redraw.take();
        if draw {
            self.renderer.draw(&self.camera, pw, ph, dpr);
            self.hash_stale = true;
        }
        if hud {
            self.update_hud();
        }
        if self.hash_stale && !animating && self.pointers.is_empty() {
            self.hash_stale = false;
            let hash = format_view_hash(&self.camera);
            if hash != self.last_hash {
                if let Ok(history) = self.window.history() {
                    let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&hash));
                }
                self.last_hash = hash;
            }
        }
    }

    fn update_hud(&self) {
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
    }

    // ---- pointer input -------------------------------------------------

    fn pointer_down(&mut self, e: &PointerEvent) {
        if e.pointer_type() == "mouse" && e.button() != 0 {
            return;
        }
        let _ = self.canvas.set_pointer_capture(e.pointer_id());
        self.stop_motion();
        self.pointers.retain(|p| p.id != e.pointer_id());
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
        // The cursor readout alone never needs a redraw of the globe.
        self.redraw.mark_hud();

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

    let app = Rc::new(RefCell::new(App {
        window: window.clone(),
        canvas: canvas.clone(),
        renderer,
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
        redraw: RedrawFlags::new(),
        hash_stale: false,
        last_hash: String::new(),
        hud_cursor: element(&doc, "hud-cursor")?,
        hud_center: element(&doc, "hud-center")?,
        hud_alt: element(&doc, "hud-alt")?,
    }));

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
            if let Some((lat, lon)) = a.camera.pick(e.client_x() as f64, e.client_y() as f64) {
                let alt = a.camera.alt * if e.shift_key() { 2.5 } else { 0.4 };
                a.fly_to(lat, lon, alt);
            }
        })?;
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
                if let Some(c) = parse_view_hash(&hash) {
                    app.borrow_mut().fly_to(c.lat, c.lon, c.alt);
                }
            },
        )?;
    }

    // On-screen buttons.
    let buttons: [(&str, Action); 3] = [
        ("zoom-in", |a| a.zoom(0.5, None)),
        ("zoom-out", |a| a.zoom(2.0, None)),
        ("reset", |a| a.reset()),
    ];
    for (id, action) in buttons {
        let button: HtmlElement = element(&doc, id)?;
        let app = app.clone();
        listen(button.as_ref(), "click", true, move |_: MouseEvent| {
            action(&mut app.borrow_mut())
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
            Ok(layers) => app.borrow_mut().renderer.upload_map(&layers),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => {
                let _ = loading.class_list().add_1("hidden");
                app.borrow_mut().redraw.mark_render();
            }
            Err(e) => {
                let msg = e.as_string().unwrap_or_else(|| format!("{e:?}"));
                web_sys::console::error_1(&e);
                loading.set_text_content(Some(&format!("Failed to load map data: {msg}")));
                let _ = loading.class_list().add_1("error");
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
