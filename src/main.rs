//! Maps — an interactive globe written in Rust and compiled to WebAssembly.

mod camera;
mod controls;
mod country;
mod country_cycle;
mod credits;
mod credits_ui;
mod data;
mod ephemeris;
mod geo;
mod geodesic;
mod imagery;
mod info_panel;
mod label_layer;
mod labels;
mod layer_loader;
mod legend_ui;
mod lines;
mod measure;
mod overlay;
mod projection;
mod quality;
mod render;
mod scrub;
mod scrub_ui;
mod search;
mod search_ui;
mod share;
mod sun;
mod thematic;
mod tile_pack;
mod tile_stream;
mod tiles;
mod view;

use camera::Camera;
use controls::{Controls, Direction, Event, Response};
use layer_loader::{Effect, LayerLoader};
use measure::Measure;
use projection::{Morph, Projection};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use view::{
    format_altitude, format_lat_lon, format_view_hash, parse_hash_layer, parse_hash_proj,
    parse_view_hash, scale_bar, share_url, Layer, RedrawFlags, SunView, Units, HOME,
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
/// The URL hash waits this long after the last time-scrubber change.
const SCRUB_HASH_DELAY_MS: f64 = 250.0;

struct App {
    window: Window,
    canvas: HtmlCanvasElement,
    renderer: render::Renderer,
    /// Which layer is shown, wanted and loading. See `layer_loader`.
    layers_state: LayerLoader<Layer>,
    /// Fetched (still encoded) data of layers that need it. Kept so a lost
    /// WebGL context can rebuild the layer's texture without a download.
    layer_data: HashMap<Layer, Rc<Vec<u8>>>,
    /// The image layer (Satellite or Terrain) whose picture is in the shared
    /// image texture, or was before a context loss.
    image_slot: Option<Layer>,
    /// Handle to this app, for spawning layer fetches.
    me: Weak<RefCell<App>>,
    layer_status: HtmlElement,
    /// Set when the last load failed; cleared by the next selection.
    layer_error: Option<String>,
    layer_buttons: Vec<(Layer, HtmlElement)>,
    /// Picker for the thematic layers; the buttons cover the others.
    theme_select: web_sys::HtmlSelectElement,
    legend: legend_ui::Legend,
    credits: credits_ui::CreditsPanel,
    sun_on: bool,
    /// Fixed time from `&t=`; `None` follows the clock.
    sun_pin: Option<f64>,
    sun_button: HtmlElement,
    scrub_ui: scrub_ui::ScrubUi,
    /// Play state for the time scrubber (`playing` is the on/off switch).
    player: scrub::Player,
    playing: bool,
    /// Frame time of the last scrub, so the URL hash waits until it stops.
    last_scrub_ms: f64,
    camera: Camera,
    /// Timeline of the globe <-> flat morph; `camera.proj` is its target.
    morph: Morph,
    proj_button: HtmlElement,
    proj_menu: HtmlElement,
    proj_items: Vec<(Projection, HtmlElement)>,
    /// Drag, pinch, inertia, zoom and flight state; mutates `camera`.
    controls: Controls,
    last_frame_ms: Option<f64>,
    /// Dynamic resolution policy and the scale of the last drawn frame.
    quality: quality::Quality,
    drawn_scale: f64,
    cursor: Option<(f64, f64)>,
    /// The last pointer was a mouse, so the cursor position means hover.
    mouse_hover: bool,
    /// The last pointer pressed or moved was touch or pen, not a mouse.
    coarse_pointer: bool,
    countries: Option<country::Countries>,
    /// Country-ID raster, kept so a restored context can re-upload it.
    ids: Option<Rc<Vec<u8>>>,
    /// Country under the mouse cursor, and the one shown in the info panel.
    hover: Option<usize>,
    selected: Option<usize>,
    /// Anchor points for country cycling, built on first use.
    country_anchors: Vec<country_cycle::Anchor>,
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
    /// Detail tiles for the image layers (see `tile_stream`).
    tile_stream: tile_stream::TileStream,
    /// Last `data-tiles` value, so the attribute is only written on change.
    tiles_attr: &'static str,
}

impl App {
    fn context_lost(&mut self) {
        self.renderer.context_lost();
        self.tile_stream.context_lost();
        self.sync_imagery_attr();
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
                self.sync_imagery_attr();
                self.restore_imagery();
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

    /// The user picked a projection (picker, key, URL). The camera switches
    /// at once (picking and panning follow the new map) while the picture
    /// morphs, or jumps when the user prefers reduced motion.
    fn set_projection(&mut self, proj: Projection) {
        for (p, b) in &self.proj_items {
            let _ = b.set_attribute("aria-pressed", if *p == proj { "true" } else { "false" });
        }
        let _ = self.proj_button.set_attribute(
            "title",
            &format!("Projection: {} (P cycles, G globe)", proj.label()),
        );
        let _ = self
            .proj_button
            .set_attribute("aria-label", &format!("Map projection: {}", proj.label()));
        let _ = self.canvas.set_attribute(
            "aria-label",
            &format!(
                "Interactive map, {} projection. Drag to pan, scroll to zoom.",
                proj.label()
            ),
        );
        if proj == self.camera.proj {
            return;
        }
        let reduced = self
            .window
            .match_media("(prefers-reduced-motion: reduce)")
            .ok()
            .flatten()
            .is_some_and(|m| m.matches());
        self.camera.set_projection(proj);
        self.morph.start(proj, reduced);
        self.update_hover();
        self.redraw.mark_render();
        self.hash_stale = true;
    }

    /// Open or close the projection menu.
    fn set_proj_menu(&mut self, open: bool) {
        self.proj_menu.set_hidden(!open);
        let _ = self
            .proj_button
            .set_attribute("aria-expanded", if open { "true" } else { "false" });
        if open {
            // Keyboard users land on the current choice.
            let current = self.camera.proj;
            if let Some((_, b)) = self.proj_items.iter().find(|(p, _)| *p == current) {
                let _ = b.focus();
            }
        }
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
                    self.sync_theme();
                    self.redraw.mark_render();
                    // Lets the visual tests wait for the layer to be on screen.
                    self.set_globe_attr("data-layer", l.id());
                }
                Effect::Fetch(l) => self.fetch_layer(l),
                Effect::Failed(l, msg) => {
                    web_sys::console::error_1(&JsValue::from_str(&msg));
                    self.layer_error =
                        Some(format!("Could not load {}. Select it to retry.", l.label()));
                }
            }
        }
        if self.layers_state.pending().is_some() {
            self.layer_error = None;
        }
        self.refresh_layer_ui();
        self.hash_stale = true;
    }

    /// Point the renderer, the legend and the info panel at the shown layer's
    /// theme. Runs when the layer changes and when the country data arrives.
    fn sync_theme(&mut self) {
        let theme = self.layers_state.shown().theme();
        let classes = theme
            .zip(self.countries.as_ref())
            .map(|(t, c)| c.theme_classes(t));
        self.renderer.set_theme_classes(classes.as_ref());
        let year = self.countries.as_ref().map_or(0, |c| c.data_year());
        self.legend.show(theme, year);
        self.credits.show_layer(self.layers_state.shown());
        // The open info panel gains or loses the theme's value row.
        let c = self.selected.and_then(|i| self.countries.as_ref()?.get(i));
        if c.is_some() {
            self.info.show(c, theme);
        }
        self.redraw.mark_render();
    }

    fn fetch_layer(&mut self, layer: Layer) {
        let Some(app) = self.me.upgrade() else { return };
        let Some(name) = layer.data_file() else {
            // Cannot happen: only layers with data are fetched.
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            let result = Self::load_layer_data(&app, layer, name).await;
            let mut a = app.borrow_mut();
            let in_gpu = result == Ok(true);
            let effects = a.layers_state.finished(layer, result.map(|_| ()));
            if !in_gpu {
                // Stale result: the bytes are cached but not on the GPU, so
                // the next selection must upload them before showing.
                a.layers_state.evict(layer);
            }
            a.apply_layer_effects(effects);
        });
    }

    /// Fetch (or take from the cache), decode and upload one image layer.
    /// Image layers share one texture, so this replaces the other's picture.
    /// Returns whether the image went to the GPU: a layer the user has moved
    /// away from meanwhile is cached but not uploaded, so the shown layer
    /// keeps its picture. No borrow of the app is held across an await.
    async fn load_layer_data(
        app: &Rc<RefCell<App>>,
        layer: Layer,
        name: &str,
    ) -> Result<bool, String> {
        let js_msg = |e: JsValue| e.as_string().unwrap_or_else(|| format!("{e:?}"));
        debug_assert!(layer.is_image(), "only image layers have data");
        let cached = app.borrow().layer_data.get(&layer).cloned();
        let bytes = match cached {
            Some(b) => b,
            None => Rc::new(data::fetch(name).await.map_err(js_msg)?),
        };
        app.borrow_mut().layer_data.insert(layer, bytes.clone());
        if app.borrow().layers_state.wanted() != layer {
            return Ok(false);
        }
        let width = app.borrow().renderer.imagery_width();
        let bitmap = imagery::decode(&bytes, width).await.map_err(js_msg)?;
        let mut a = app.borrow_mut();
        if a.layers_state.wanted() != layer {
            bitmap.close();
            return Ok(false);
        }
        // A no-op while the context is lost; `context_restored` redoes it.
        let uploaded = a.renderer.upload_imagery(&bitmap);
        bitmap.close();
        uploaded.map_err(js_msg)?;
        a.set_image_slot(layer);
        Ok(true)
    }

    /// `layer`'s picture is now in the shared image texture: any other image
    /// layer must load again before it is shown.
    fn set_image_slot(&mut self, layer: Layer) {
        if let Some(old) = self.image_slot.filter(|&l| l != layer) {
            self.layers_state.evict(old);
        }
        self.image_slot = Some(layer);
        self.sync_imagery_attr();
    }

    /// After a context restore: decode the kept bytes of the image layer that
    /// was in the texture again and upload them.
    fn restore_imagery(&mut self) {
        let Some(layer) = self.image_slot else { return };
        let Some(bytes) = self.layer_data.get(&layer).cloned() else {
            return;
        };
        let Some(app) = self.me.upgrade() else { return };
        let width = self.renderer.imagery_width();
        wasm_bindgen_futures::spawn_local(async move {
            let bitmap = match imagery::decode(&bytes, width).await {
                Ok(b) => b,
                Err(e) => return web_sys::console::error_1(&e),
            };
            let mut a = app.borrow_mut();
            // Another image layer may have taken the texture meanwhile.
            if a.image_slot == Some(layer) {
                if let Err(e) = a.renderer.upload_imagery(&bitmap) {
                    web_sys::console::error_1(&e);
                }
                a.sync_imagery_attr();
                a.redraw.mark_render();
            }
            bitmap.close();
        });
    }

    /// Choose and request detail tiles for the view about to be drawn, and
    /// give the renderer a changed page table. See `docs/tile-streaming.md`.
    fn update_tiles(&mut self, dpr: f64) {
        if self.renderer.is_lost() {
            return;
        }
        let shown = self.layers_state.shown();
        let base = if shown.is_image() && self.image_slot == Some(shown) {
            self.renderer.imagery_level()
        } else {
            None
        };
        let cam = &self.camera;
        let plan = self
            .tile_stream
            .plan(base.map(|b| (shown.id(), b)), |levels, cap| {
                tiles::select(cam, dpr, levels, cap)
            });
        for d in plan.dirs {
            self.fetch_tile_dir(d);
        }
        for f in plan.tiles {
            self.fetch_tile(f);
        }
        if let Some(update) = self.tile_stream.take_pages() {
            let pages = update.as_ref().map(|(l, t)| (*l, t.as_slice()));
            if let Err(e) = self.renderer.set_tiles(pages) {
                web_sys::console::error_1(&e);
            }
        }
        // Lets the visual tests wait for the tiles.
        let status = self.tile_stream.status();
        if status != self.tiles_attr {
            self.tiles_attr = status;
            self.set_globe_attr("data-tiles", status);
        }
    }

    fn fetch_tile_dir(&self, d: tile_stream::DirFetch) {
        let Some(app) = self.me.upgrade() else { return };
        wasm_bindgen_futures::spawn_local(async move {
            let bytes = tile_stream::fetch_range(&d.url, d.offset, d.len)
                .await
                .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")));
            let mut a = app.borrow_mut();
            let error = match d.page {
                None => {
                    let dir = bytes
                        .and_then(|b| tile_pack::Directory::parse(&b, d.min, d.max, d.entries));
                    let error = dir.as_ref().err().cloned();
                    a.tile_stream.dir_loaded(d.set, dir);
                    error.map(|e| format!("no detail tiles from {}: {e}", d.url))
                }
                Some(page) => a
                    .tile_stream
                    .page_loaded(d.set, page, bytes)
                    .err()
                    .map(|e| format!("{}: directory page {page}: {e}", d.url)),
            };
            if let Some(msg) = error {
                web_sys::console::warn_1(&msg.into());
            }
            a.redraw.mark_render();
        });
    }

    fn fetch_tile(&self, f: tile_stream::TileFetch) {
        let Some(app) = self.me.upgrade() else { return };
        wasm_bindgen_futures::spawn_local(async move {
            let bitmap = match tile_stream::fetch_range(&f.url, f.offset, f.len).await {
                Ok(bytes) => tile_stream::decode_tile(&bytes).await,
                Err(e) => Err(e),
            };
            let mut a = app.borrow_mut();
            let failed = match bitmap {
                Ok(bitmap) => {
                    let uploaded = match a.tile_stream.tile_arrived(&f) {
                        Some(slot) => a.renderer.upload_tile(slot, &bitmap).map(|_| ()),
                        None => Ok(()),
                    };
                    bitmap.close();
                    uploaded.err()
                }
                Err(e) => Some(e),
            };
            if let Some(e) = failed {
                let msg = format!(
                    "tile {:?} failed: {}",
                    f.id,
                    e.as_string().unwrap_or_else(|| format!("{e:?}"))
                );
                web_sys::console::warn_1(&msg.into());
                a.tile_stream.tile_failed(&f);
            }
            a.redraw.mark_render();
        });
    }

    fn set_globe_attr(&self, name: &str, value: &str) {
        let _ = self.canvas.set_attribute(name, value);
    }

    /// `data-imagery` on the canvas tells the tests whether the satellite
    /// texture is in GPU memory.
    fn sync_imagery_attr(&self) {
        let on = self.renderer.has_imagery();
        self.set_globe_attr("data-imagery", if on { "1" } else { "0" });
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
        // The select shows the wanted layer when it is a thematic one.
        self.theme_select.set_value(if wanted.theme().is_some() {
            wanted.id()
        } else {
            ""
        });
        let text = match (pending, &self.layer_error) {
            (Some(l), _) => format!("Loading {}…", l.label()),
            (None, Some(e)) => e.clone(),
            _ => String::new(),
        };
        self.layer_status.set_text_content(Some(&text));
        self.layer_status.set_hidden(text.is_empty());
    }

    /// Push the current sun direction (or none when off) to the renderer
    /// and schedule a redraw.
    fn refresh_sun(&mut self) {
        let t = self.sun_time();
        let dir = if self.sun_on {
            sun::sun_direction(t)
        } else {
            None
        };
        self.renderer.set_sun(dir);
        self.scrub_ui.show(self.sun_on);
        if self.sun_on {
            self.scrub_ui
                .sync(t, self.sun_pin.is_none(), self.playing, self.player.mode);
        }
        let _ = self
            .sun_button
            .set_attribute("aria-pressed", if self.sun_on { "true" } else { "false" });
        self.redraw.mark_render();
    }

    /// The time the terminator shows: the pin, or the clock.
    fn sun_time(&self) -> f64 {
        self.sun_pin.unwrap_or_else(|| js_sys::Date::now() / 1000.0)
    }

    /// Pin the terminator to `t` (whole seconds, inside the scrubber's range).
    fn pin_time(&mut self, t: f64) {
        self.sun_pin = Some(scrub::clamp_t(t.floor()));
        self.refresh_sun();
        self.hash_stale = true;
    }

    /// A manual change (slider or key): stops play and pins the new time.
    fn scrub_to(&mut self, t: f64) {
        self.playing = false;
        self.last_scrub_ms = self.last_frame_ms.unwrap_or(0.0);
        self.pin_time(t);
    }

    /// Step the pinned time (or the clock, when live) by `delta` seconds.
    fn scrub_by(&mut self, delta: f64) {
        if self.sun_on {
            self.scrub_to(scrub::shift(self.sun_time(), delta));
        }
    }

    /// Back to the live clock.
    fn go_live(&mut self) {
        self.playing = false;
        self.sun_pin = None;
        self.refresh_sun();
        self.hash_stale = true;
    }

    fn set_playing(&mut self, play: bool) {
        let play = play && self.sun_on;
        if play && !self.playing {
            let reduced = self
                .window
                .match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
                .is_some_and(|m| m.matches());
            self.player = scrub::Player::new(self.player.mode, reduced);
            // Playing from the live clock pins the time where it is.
            self.sun_pin = Some(scrub::clamp_t(self.sun_time().floor()));
        }
        if !play && self.playing {
            // Whole seconds, so the URL and share link stay short.
            if let Some(t) = self.sun_pin {
                self.sun_pin = Some(t.floor());
            }
            self.hash_stale = true;
        }
        self.playing = play;
        self.refresh_sun();
    }

    fn toggle_sun(&mut self) {
        self.sun_on = !self.sun_on;
        self.playing = self.playing && self.sun_on;
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

    /// Backspace: remove the selected point, or the last one.
    fn measure_undo(&mut self) {
        if self.measure.active && self.measure.undo() {
            self.measure_changed();
            self.update_hover();
        }
    }

    /// Delete: remove the selected point only.
    fn measure_delete(&mut self) {
        if self.measure.active && self.measure.remove_selected() {
            self.measure_changed();
            self.update_hover();
        }
    }

    /// Hover state of the measure points under a mouse cursor. Cheap, and a
    /// redraw happens only when it changes.
    fn update_point_hover(&mut self) {
        let hit = match self.cursor {
            Some((x, y)) if self.mouse_hover && self.measure.active => measure::hit_test(
                &self.camera,
                self.measure.points(),
                x,
                y,
                measure::hit_radius(false),
            ),
            _ => None,
        };
        if self.measure.set_hover(hit) {
            self.redraw.mark_render();
        }
        let _ = self
            .canvas
            .class_list()
            .toggle_with_force("over-point", hit.is_some());
    }

    /// Whether a plain click or tap on the globe opens the country panel.
    /// This is the one place to return false while another tool owns taps
    /// on the globe (measure mode places points instead).
    fn info_click_enabled(&self) -> bool {
        !self.measure.active
    }

    /// Country at a screen point, once the country data has loaded.
    /// A country whose outline is within a few pixels counts, so tiny
    /// islands can be hovered and tapped; `coarse` widens that for touch.
    fn country_at(&self, x: f64, y: f64, coarse: bool) -> Option<usize> {
        self.countries
            .as_ref()?
            .hit_near(&self.camera, x, y, country::near_radius(coarse))
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
        self.update_point_hover();
        let hit = match self.cursor {
            Some((x, y)) if self.mouse_hover && !self.measure.active => {
                self.country_at(x, y, false)
            }
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
        self.info.show(c, self.layers_state.shown().theme());
        self.sync_highlight();
    }

    /// Tab, Shift+Tab, `[` and `]`: select the next or previous visible
    /// country (reading order). The view does not move: only countries whose
    /// anchor is on screen take part, so the target is always in view. Without
    /// `wrap`, stepping past either end closes the panel and returns false so
    /// the key is left alone and focus moves on.
    fn cycle_country(&mut self, forward: bool, wrap: bool) -> bool {
        let Some(countries) = self.countries.as_ref() else {
            return false;
        };
        if self.country_anchors.is_empty() {
            self.country_anchors = countries.anchors();
        }
        let order = country_cycle::visible_order(&self.country_anchors, &self.camera);
        match country_cycle::step(&order, self.selected, forward, wrap) {
            country_cycle::Step::To(i) => {
                self.select(Some(i));
                true
            }
            country_cycle::Step::Release => {
                self.select(None);
                false
            }
        }
    }

    fn click(&mut self, x: f64, y: f64) {
        if self.info_click_enabled() {
            let hit = self.country_at(x, y, self.coarse_pointer);
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

        // Play moves the pinned time every frame; nothing else here runs
        // while it is paused or live.
        if self.playing {
            let t = self.sun_time();
            let next = self.player.advance(t, dt);
            if next != t {
                self.sun_pin = Some(next);
                self.refresh_sun();
            }
        }
        let tick = self.send(Event::Tick { dt });
        let animating = tick.render || self.playing;
        if let Some((x, y)) = tick.single_tap {
            if !self.measure.active {
                self.click(x, y);
            }
        }
        if animating {
            self.update_hover(); // the globe moved under a still cursor
        }
        // The morph counts as motion: it draws at the reduced scale and, like
        // a flight, holds back the URL hash until it ends.
        let morphing = self.morph.tick(dt);
        if morphing {
            self.redraw.mark_render();
        }
        let animating = animating || self.morph.animating();
        // Dynamic resolution: draw at a lower scale while the view moves, and
        // once more at full resolution after it has settled.
        let moving =
            self.controls.in_motion(&self.camera) || self.morph.animating() || self.playing;
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
        // Labels and the measure path sit at fixed screen positions, which are
        // wrong while the map is between projections: hide them until it lands.
        let morphing = self.morph.animating();
        if morphing {
            self.labels.clear();
            self.overlay.clear();
        } else if draw || relayout || self.labels.animating() {
            self.labels.update(&self.camera, dt, pw, ph, dpr);
        }
        if draw {
            self.update_tiles(dpr);
            self.renderer
                .draw(&self.camera, self.morph.blend(), pw, ph, dpr, scale);
            self.drawn_scale = scale;
            if !morphing {
                self.overlay.draw(&self.camera, &self.measure, pw, ph, dpr);
            }
            self.hash_stale = true;
        }
        if hud {
            self.update_hud();
        }
        // A slider drag changes the pin on every input event: wait until it pauses.
        let scrubbing = now_ms - self.last_scrub_ms < SCRUB_HASH_DELAY_MS;
        if self.hash_stale && !animating && !self.controls.dragging() && !scrubbing {
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
            .set_text_content(Some(&format_altitude(self.camera.alt, self.units)));
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

    /// Switch to the next unit, remember it, and refresh the HUD text (the globe does
    /// not change, so no redraw).
    fn cycle_units(&mut self) {
        self.units = self.units.next();
        save_units(&self.window, self.units);
        self.redraw.mark_hud();
    }

    // ---- pointer input -------------------------------------------------

    fn pointer_down(&mut self, e: &PointerEvent) {
        if e.pointer_type() == "mouse" && e.button() != 0 {
            return;
        }
        if !self.proj_menu.hidden() {
            self.set_proj_menu(false);
        }
        self.coarse_pointer = e.pointer_type() != "mouse";
        let _ = self.canvas.set_pointer_capture(e.pointer_id());
        // A press on a measure point grabs it instead of panning the view.
        // Shift+click skips this and places a point even next to an
        // existing one.
        if self.measure.active && !e.shift_key() {
            let (x, y) = (e.client_x() as f64, e.client_y() as f64);
            let radius = measure::hit_radius(e.pointer_type() != "mouse");
            if self
                .measure
                .press(&self.camera, e.pointer_id(), x, y, radius)
            {
                self.redraw.mark_render();
                return;
            }
        }
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
        if e.pointer_type() == "mouse"
            && view::is_phantom_mouse_move(
                self.cursor.is_some(),
                e.buttons(),
                e.movement_x() as f64,
                e.movement_y() as f64,
            )
        {
            return;
        }
        let (x, y) = (e.client_x() as f64, e.client_y() as f64);
        self.cursor = Some((x, y));
        self.mouse_hover = e.pointer_type() == "mouse";
        self.coarse_pointer = !self.mouse_hover;
        // The cursor readout alone never needs a redraw of the globe.
        self.redraw.mark_hud();
        if self.measure.owns(e.pointer_id()) {
            // Dragging a measure point: the view stays put.
            if self.measure.drag(&self.camera, e.pointer_id(), x, y) {
                let _ = self.canvas.class_list().add_1("dragging");
                self.redraw.mark_render();
            }
            self.update_hover();
            return;
        }
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
        if self.measure.owns(e.pointer_id()) {
            let r = self
                .measure
                .release(e.pointer_id(), t, e.type_() != "pointerup");
            match r {
                measure::Release::Removed => self.measure_changed(),
                measure::Release::Add(x, y) => self.measure_add_at(x, y),
                _ => {}
            }
            self.redraw.mark_render();
            if !self.controls.dragging() {
                let _ = self.canvas.class_list().remove_1("dragging");
            }
            self.update_hover();
            return;
        }
        let r = self.send(Event::PointerUp {
            id: e.pointer_id(),
            t,
            cancel: e.type_() != "pointerup",
        });
        // The single tap detector: a lone press released near where it went
        // down. Measure mode places a point at once. Country info waits for
        // `single_tap` so a double-click (fly-in) does not open the panel.
        if let Some((x, y)) = r.tap {
            if self.measure.active {
                self.measure_add_at(x, y);
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

const UNITS_KEY: &str = "biruni.units";

/// Saved unit, or km when nothing usable is stored. `localStorage` can be
/// missing or throw (private mode, blocked site data), so every step falls
/// back instead of failing.
fn load_units(window: &Window) -> Units {
    window
        .local_storage()
        .ok()
        .flatten()
        .and_then(|s| s.get_item(UNITS_KEY).ok().flatten())
        .and_then(|id| Units::from_id(&id))
        .unwrap_or(Units::Metric)
}

fn save_units(window: &Window, units: Units) {
    if let Ok(Some(s)) = window.local_storage() {
        let _ = s.set_item(UNITS_KEY, units.id());
    }
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

    let initial_proj = camera.proj;
    let mut proj_items = Vec::new();
    for p in Projection::ALL {
        let b: HtmlElement = doc
            .query_selector(&format!("#proj-menu button[data-proj=\"{}\"]", p.id()))?
            .ok_or_else(|| JsValue::from_str("missing projection menu item"))?
            .dyn_into()?;
        proj_items.push((p, b));
    }
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
    for layer in Layer::ALL.into_iter().filter(|l| l.theme().is_none()) {
        let b: HtmlElement = element(&doc, &format!("layer-{}", layer.id()))?;
        layer_buttons.push((layer, b));
    }
    let theme_select: web_sys::HtmlSelectElement = element(&doc, "layer-theme")?;

    let overlay_canvas: HtmlCanvasElement = element(&doc, "overlay")?;
    let overlay = overlay::Overlay::new(overlay_canvas)?;
    let labels = label_layer::LabelLayer::new(element(&doc, "labels")?)?;
    let measure_panel = overlay::Panel::new(&doc)?;

    let renderer_tile_capacity = renderer.tile_capacity();
    let app = Rc::new(RefCell::new(App {
        window: window.clone(),
        canvas: canvas.clone(),
        renderer,
        layers_state: LayerLoader::new(Layer::default()),
        layer_data: HashMap::new(),
        image_slot: None,
        me: Weak::new(),
        layer_status: element(&doc, "layer-status")?,
        layer_error: None,
        layer_buttons: layer_buttons.clone(),
        theme_select: theme_select.clone(),
        legend: legend_ui::Legend::new(&doc)?,
        credits: credits_ui::CreditsPanel::new(&doc)?,
        sun_on: sun_hash.on,
        sun_pin: sun_hash.pin,
        sun_button: element(&doc, "sun-toggle")?,
        scrub_ui: scrub_ui::ScrubUi::new(&doc)?,
        player: scrub::Player::default(),
        playing: false,
        last_scrub_ms: f64::NEG_INFINITY,
        camera,
        morph: Morph::new(initial_proj),
        proj_button: element(&doc, "proj-button")?,
        proj_menu: element(&doc, "proj-menu")?,
        proj_items: proj_items.clone(),
        controls: Controls::new(&camera),
        last_frame_ms: None,
        quality: quality::Quality::new(quality::parse_setting(
            &window.location().search().unwrap_or_default(),
        )),
        drawn_scale: 1.0,
        cursor: None,
        mouse_hover: false,
        coarse_pointer: false,
        countries: None,
        ids: None,
        hover: None,
        selected: None,
        country_anchors: Vec::new(),
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
        units: load_units(&window),
        layers: None,
        status: loading.clone(),
        share_ui: share::ShareUi::new(&window, &doc, canvas.clone().into())?,
        measure: Measure::default(),
        labels,
        keepout: label_layer::KeepOut::new(&doc)?,
        overlay,
        measure_panel,
        tile_stream: tile_stream::TileStream::new(renderer_tile_capacity),
        tiles_attr: "",
    }));

    {
        let mut a = app.borrow_mut();
        a.me = Rc::downgrade(&app);
        // The URL layer takes the same path as a click.
        a.select_layer(initial_layer);
    }
    app.borrow_mut().refresh_sun();
    {
        // Show the projection from the URL in the picker, and wire it.
        let mut a = app.borrow_mut();
        a.set_projection(initial_proj); // already current: only labels the picker and canvas
        let button: EventTarget = a.proj_button.clone().into();
        drop(a);
        {
            let app = app.clone();
            listen(&button, "click", true, move |_: MouseEvent| {
                let mut a = app.borrow_mut();
                let open = a.proj_menu.hidden();
                a.set_proj_menu(open);
            })?;
        }
        for (p, b) in proj_items {
            let app = app.clone();
            listen(b.as_ref(), "click", true, move |_: MouseEvent| {
                let mut a = app.borrow_mut();
                a.set_projection(p);
                a.set_proj_menu(false);
                let _ = a.proj_button.focus();
            })?;
        }
    }

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
    // Time scrubber: sliders pin the time, play animates it, Now goes live.
    {
        let (time, date, play, mode, now) = {
            let a = app.borrow();
            let u = &a.scrub_ui;
            (
                u.time_input().clone(),
                u.date_input().clone(),
                u.play_button().clone(),
                u.mode_button().clone(),
                u.now_button().clone(),
            )
        };
        {
            let app = app.clone();
            listen(time.as_ref(), "input", true, move |_: web_sys::Event| {
                let mut a = app.borrow_mut();
                let t = scrub::with_minute_of_day(a.sun_time(), a.scrub_ui.minute());
                a.scrub_to(t);
            })?;
        }
        {
            let app = app.clone();
            listen(date.as_ref(), "input", true, move |_: web_sys::Event| {
                let mut a = app.borrow_mut();
                let t = scrub::with_day_of_year(a.sun_time(), a.scrub_ui.day());
                a.scrub_to(t);
            })?;
        }
        {
            let app = app.clone();
            listen(play.as_ref(), "click", true, move |_: MouseEvent| {
                let mut a = app.borrow_mut();
                let playing = a.playing;
                a.set_playing(!playing);
            })?;
        }
        {
            let app = app.clone();
            listen(mode.as_ref(), "click", true, move |_: MouseEvent| {
                let mut a = app.borrow_mut();
                let reduced = a.player.reduced;
                a.player = scrub::Player::new(a.player.mode.toggled(), reduced);
                a.refresh_sun();
            })?;
        }
        {
            let app = app.clone();
            listen(now.as_ref(), "click", true, move |_: MouseEvent| {
                app.borrow_mut().go_live();
            })?;
        }
    }
    {
        let app = app.clone();
        let select = theme_select.clone();
        listen(
            select.clone().as_ref(),
            "change",
            true,
            move |_: web_sys::Event| {
                // The empty option is only a prompt: leave the layer alone.
                if let Some(layer) = Layer::from_id(&select.value()) {
                    app.borrow_mut().select_layer(layer);
                }
            },
        )?;
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

    // Clicking or tapping the scale bar cycles km, mi and nmi.
    {
        let app = app.clone();
        let scale: EventTarget = app.borrow().hud_scale.clone().into();
        listen(&scale, "click", true, move |_: MouseEvent| {
            let mut a = app.borrow_mut();
            a.cycle_units();
        })?;
    }

    // The credits button toggles the panel; its × button closes it.
    {
        let (button, close): (EventTarget, EventTarget) = {
            let a = app.borrow();
            (
                a.credits.button.clone().into(),
                a.credits.close.clone().into(),
            )
        };
        let a1 = app.clone();
        listen(&button, "click", true, move |_: MouseEvent| {
            let a = a1.borrow();
            let open = !a.credits.is_open();
            a.credits.set_open(open);
        })?;
        let a2 = app.clone();
        listen(&close, "click", true, move |_: MouseEvent| {
            a2.borrow().credits.set_open(false);
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
        // Not passive: Tab in measure mode calls preventDefault.
        listen(
            window.as_ref(),
            "keydown",
            false,
            move |e: KeyboardEvent| {
                // Typing in a text field (the search box) is not a map shortcut.
                if e.target().is_some_and(|t| {
                    t.is_instance_of::<web_sys::HtmlInputElement>()
                        || t.is_instance_of::<web_sys::HtmlTextAreaElement>()
                }) {
                    return;
                }
                let mut a = app.borrow_mut();
                // The credits panel: Esc closes it; keys typed inside it
                // (Tab, Enter on a link) are not map shortcuts.
                if a.credits.is_open() && e.key() == "Escape" {
                    a.credits.set_open(false);
                    return;
                }
                if e.target().is_some_and(|t| a.credits.contains(&t)) {
                    return;
                }
                // Measure editing keys. Shift+arrows nudge the selected point;
                // without a selection they pan as usual.
                if a.measure.active {
                    let on_page = e.target().is_some_and(|t| {
                        let c: &EventTarget = a.canvas.as_ref();
                        let body: Option<EventTarget> =
                            a.window.document().and_then(|d| d.body()).map(Into::into);
                        &t == c || Some(&t) == body.as_ref()
                    });
                    let plain = !(e.ctrl_key() || e.meta_key() || e.alt_key());
                    let step = measure::NUDGE_PX;
                    let nudge = match e.key().as_str() {
                        "ArrowLeft" => Some((-step, 0.0)),
                        "ArrowRight" => Some((step, 0.0)),
                        "ArrowUp" => Some((0.0, -step)),
                        "ArrowDown" => Some((0.0, step)),
                        _ => None,
                    };
                    let consumed = match (e.key().as_str(), nudge) {
                        (_, Some((dx, dy))) if e.shift_key() && plain => {
                            a.measure.selected().is_some() && {
                                let cam = a.camera;
                                a.measure.nudge_selected(&cam, dx, dy);
                                true
                            }
                        }
                        ("[", _) if plain => a.measure.cycle_selection(false),
                        ("]", _) if plain => a.measure.cycle_selection(true),
                        ("Tab", _) if plain && on_page => a.measure.tab_selection(!e.shift_key()),
                        ("Delete", _) if plain => {
                            a.measure_delete();
                            true
                        }
                        _ => false,
                    };
                    if consumed {
                        e.prevent_default();
                        a.redraw.mark_render();
                        return;
                    }
                }
                // Country cycling. Tab only counts when focus is on the page
                // itself (never steals it from a button) and is released at
                // the ends so it cannot trap focus; `[` and `]` wrap.
                if !a.measure.active
                    && !(e.ctrl_key() || e.meta_key() || e.alt_key())
                    && a.info_click_enabled()
                {
                    let key = e.key();
                    let on_page = e.target().is_some_and(|t| {
                        let c: &EventTarget = a.canvas.as_ref();
                        let body: Option<EventTarget> =
                            a.window.document().and_then(|d| d.body()).map(Into::into);
                        &t == c || Some(&t) == body.as_ref()
                    });
                    let moved = match key.as_str() {
                        "Tab" if on_page => Some(a.cycle_country(!e.shift_key(), false)),
                        "]" => Some(a.cycle_country(true, true)),
                        "[" => Some(a.cycle_country(false, true)),
                        _ => None,
                    };
                    match moved {
                        Some(true) => {
                            e.prevent_default();
                            a.redraw.mark_render();
                            return;
                        }
                        Some(false) => return,
                        None => {}
                    }
                }
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
                    "Home" if !(e.ctrl_key() || e.meta_key() || e.alt_key()) => {
                        a.send(Event::FlyToPole { north: true });
                    }
                    "End" if !(e.ctrl_key() || e.meta_key() || e.alt_key()) => {
                        a.send(Event::FlyToPole { north: false });
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
                        // Info for the country at the center of the view,
                        // with the same few pixels of slack as the mouse so
                        // a tiny island under the crosshair counts.
                        let (cx, cy) = (a.camera.width / 2.0, a.camera.height / 2.0);
                        let hit = a.country_at(cx, cy, false);
                        a.select(hit);
                        return;
                    }
                    // Debug aid, only with ?debug=ctxloss: lose / restore the
                    // WebGL context (see README).
                    "`" => {
                        a.renderer.debug_toggle_context_loss();
                        return;
                    }
                    "u" | "U" => {
                        // Only the HUD text changes; no need to redraw the globe.
                        a.cycle_units();
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
                    // Time scrubber (terminator on): , and . step an hour back and
                    // forward, Shift (< and >) a day; T plays and pauses.
                    "," | "." | "<" | ">"
                        if a.sun_on && !(e.ctrl_key() || e.meta_key() || e.alt_key()) =>
                    {
                        let key = e.key();
                        let step = if e.shift_key() || key == "<" || key == ">" {
                            scrub::DAY
                        } else {
                            scrub::HOUR
                        };
                        a.scrub_by(if key == "," || key == "<" {
                            -step
                        } else {
                            step
                        });
                    }
                    "t" | "T" if a.sun_on && !(e.ctrl_key() || e.meta_key() || e.alt_key()) => {
                        let playing = a.playing;
                        a.set_playing(!playing);
                    }
                    "p" | "P" if !(e.ctrl_key() || e.meta_key() || e.alt_key()) => {
                        // P cycles forward through the projections, Shift+P back.
                        let cur = a.camera.proj;
                        a.set_projection(if e.shift_key() {
                            cur.prev()
                        } else {
                            cur.next()
                        });
                    }
                    "g" | "G" if !(e.ctrl_key() || e.meta_key() || e.alt_key()) => {
                        a.set_projection(Projection::Globe)
                    }
                    "m" | "M" => a.toggle_measure(),
                    "Escape" if !a.proj_menu.hidden() => {
                        a.set_proj_menu(false);
                        let _ = a.proj_button.focus();
                        return;
                    }
                    // Esc first deselects a point, then clears and exits.
                    "Escape" if a.measure.active && a.measure.select(None) => {}
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
                    k @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8") => {
                        let i = k.parse::<usize>().unwrap_or(1) - 1;
                        if let Some(l) = Layer::ALL.get(i) {
                            a.select_layer(*l);
                        }
                    }
                    _ => return,
                }
                a.redraw.mark_render();
            },
        )?;
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
                a.set_projection(parse_hash_proj(&hash));
                let sun = SunView::from_hash(&hash);
                a.sun_on = sun.on;
                a.sun_pin = sun.pin;
                a.playing = false;
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

    // The tile pack list; without it the image layers show only their bases.
    {
        let app = app.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match tile_stream::load_tilesets().await {
                Ok(sets) => {
                    let mut a = app.borrow_mut();
                    a.tile_stream.set_tilesets(sets);
                    a.redraw.mark_render();
                }
                Err(e) => web_sys::console::warn_1(&e),
            }
        });
    }

    // Load and rasterize the map data in the background.
    let map_width = app.borrow().renderer.map_width();
    wasm_bindgen_futures::spawn_local(async move {
        let result = match geo::Layers::load(map_width).await {
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
                    let ids_width = app.borrow().renderer.ids_width();
                    // Yields between slices, so no borrow is held across it.
                    let ids = match countries.rasterize_ids_yielding(ids_width).await {
                        Ok(ids) => ids,
                        Err(e) => {
                            web_sys::console::error_1(&e);
                            return;
                        }
                    };
                    let mut a = app.borrow_mut();
                    // No-op while the context is lost; context_restored uploads it.
                    if let Err(e) = a.renderer.upload_country_ids(&ids, ids_width) {
                        web_sys::console::error_1(&e);
                    }
                    a.renderer.set_country_colors(&countries.id_colors());
                    // Thematic classes are only known now: set them up after
                    // the countries are stored, below.
                    a.redraw.mark_render(); // political fills switch on
                    a.ids = Some(Rc::new(ids));
                    a.countries = Some(countries);
                    a.sync_theme();
                    a.update_hover();
                    // Lets the visual tests wait for the political fills.
                    let globe = web_sys::window()
                        .and_then(|w| w.document())
                        .and_then(|d| d.get_element_by_id("globe"));
                    if let Some(g) = globe {
                        let _ = g.set_attribute("data-countries", "1");
                    }
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
