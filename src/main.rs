//! Maps — an interactive globe written in Rust and compiled to WebAssembly.

mod camera;
mod constellations;
mod controls;
mod country;
mod country_cycle;
mod credits;
mod credits_ui;
mod data;
mod ephemeris;
mod ethnography;
mod ethnography_app;
mod geo;
mod geodesic;
mod healpix;
mod history;
mod history_app;
mod imagery;
mod info_panel;
mod label_layer;
mod labels;
mod languages;
mod languages_app;
mod layer_loader;
mod legend_ui;
mod lines;
mod measure;
mod moon;
mod overlay;
mod paleo;
mod paleo_app;
mod projection;
mod quality;
mod render;
mod scrub;
mod scrub_ui;
mod search;
mod search_ui;
mod share;
mod sky;
mod sky_bodies;
mod sky_render;
mod solar;
mod solar_app;
mod solar_render;
mod solar_scene;
mod space;
mod stars;
mod sun;
mod survey;
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
    parse_view_hash, scale_bar, share_url, Layer, RedrawFlags, SunView, Units, View, HOME,
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
/// Width of the sky view's Milky Way image (`scripts/make_milky_way.py`),
/// and how strongly it is added to the night sky.
const MILKY_WAY_WIDTH: u32 = 2048;
const MILKY_WAY_STRENGTH: f64 = 0.3;
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
    /// Which view is shown: the Earth, the sky (`&view=sky`; the observer
    /// stands at the Earth camera's center) or the solar system
    /// (`&view=solar`). `sky_on` is `view == View::Sky`.
    view: View,
    sky_on: bool,
    sky_cam: sky::SkyCamera,
    sky_gestures: sky::Gestures,
    /// The solar system view's camera and gestures, where a press began
    /// (a tap on a body flies to it), and the bodies and names last drawn
    /// (for picking).
    solar_cam: solar::SolarCamera,
    solar_gestures: sky::Gestures,
    solar_press: Option<(f64, f64)>,
    solar_drawn: Vec<solar_scene::Drawn>,
    solar_labels: Vec<solar_scene::Label>,
    solar_select: web_sys::HtmlSelectElement,
    /// Spacecraft and small bodies (`space.rs`; fetched the first time the
    /// solar view opens), whether they show (`&objects=off` hides them),
    /// their button, and the markers last drawn (for the HUD).
    space: Option<Rc<space::Catalog>>,
    space_requested: bool,
    objects_on: bool,
    objects_button: HtmlElement,
    solar_markers: Vec<solar_scene::objects::Marker>,
    /// Surface maps of the solar view: encoded bytes (kept to decode again
    /// after eviction or a context loss), decodes under way, and failures.
    planet_bytes: HashMap<solar::Body, Rc<Vec<u8>>>,
    planet_busy: std::collections::HashSet<solar::Body>,
    planet_failed: std::collections::HashSet<solar::Body>,
    /// Earth, Sky and Solar system buttons.
    view_buttons: [HtmlElement; 3],
    /// The historical borders (`&year=`, see `history_app.rs`) and their panel.
    history: history_app::HistoryState,
    year_ui: history_app::YearUi,
    /// Deep time (`&ma=`, see `paleo_app.rs`) and its ages panel.
    paleo: paleo_app::PaleoState,
    ages_ui: paleo_app::AgesUi,
    /// The languages layer (`&languages=1`), its button, and the search box
    /// (which gets the languages as entries).
    langs: languages_app::LanguagesState,
    langs_button: HtmlElement,
    search: Option<search_ui::Search>,
    /// The societies layer (`&societies=<variable>`), its button and the
    /// variable menu in the legend.
    societies: ethnography_app::SocietiesState,
    societies_button: HtmlElement,
    ethno_select: web_sys::HtmlSelectElement,
    title_hint: HtmlElement,
    /// Labels of the first three HUD rows, which differ per view.
    hud_labels: [HtmlElement; 3],
    /// `data-sky` is set on the canvas (after the first sky frame).
    sky_attr: bool,
    /// The star catalog, once loaded (fetched the first time the sky opens).
    stars: Option<Rc<stars::Catalog>>,
    stars_requested: bool,
    /// Constellation figures (loaded with the stars) and whether they show.
    figures: Option<Rc<Vec<constellations::Constellation>>>,
    show_figures: bool,
    figures_button: HtmlElement,
    /// The Milky Way image, still encoded (decoded again after a context
    /// loss), and whether a decode is under way.
    milky_way: Option<Rc<Vec<u8>>>,
    milky_way_decoding: bool,
    /// The deep-sky survey (`survey.rs`): shown or not, the tile planner,
    /// the Allsky atlas still encoded (decoded again after a context loss),
    /// whether it is being fetched or decoded, the mirror in use, the
    /// toggle button and the last `data-survey` value.
    survey_on: bool,
    survey: survey::Survey,
    survey_allsky: Option<Rc<Vec<u8>>>,
    survey_busy: bool,
    survey_mirror: usize,
    survey_button: HtmlElement,
    survey_attr: &'static str,
}

impl App {
    fn context_lost(&mut self) {
        self.renderer.context_lost();
        self.tile_stream.context_lost();
        self.survey.context_lost();
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
                if self.history.on {
                    self.history_reapply();
                }
                if self.paleo.on {
                    self.paleo_reapply();
                }
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
        if self.history.on {
            self.history_reapply(); // the upload put the modern borders back
        }
        if self.paleo.on {
            self.paleo_reapply();
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
        self.sync_legend();
        self.credits_for_layer();
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
        self.scrub_ui.show(self.time_shown());
        if self.time_shown() {
            self.scrub_ui
                .sync(t, self.sun_pin.is_none(), self.playing, self.player.mode);
        }
        let _ = self
            .sun_button
            .set_attribute("aria-pressed", if self.sun_on { "true" } else { "false" });
        self.redraw.mark_render();
    }

    /// The time scrubber is shown: the terminator is on, or the sky view
    /// (which always depends on the time).
    fn time_shown(&self) -> bool {
        self.sun_on || self.view != View::Earth
    }

    /// The time the terminator and the sky show: the pin, or the clock.
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
        if self.time_shown() {
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
        let play = play && self.time_shown();
        if play && !self.playing {
            let reduced = self
                .window
                .match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
                .is_some_and(|m| m.matches());
            self.player = scrub::Player::new(self.player.mode, reduced);
            self.player.continuous = self.view == View::Solar;
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
        self.playing = self.playing && self.time_shown();
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
        if self.view != View::Earth
            || self.history.on
            || self.paleo.on
            || self.langs.on
            || self.societies.on
        {
            let base = page.split('#').next().unwrap_or("");
            return format!("{base}{}", self.view_hash());
        }
        share_url(
            &page,
            &self.camera,
            self.layers_state.wanted(),
            self.sun_view(),
        )
    }

    /// The URL hash for the current view. The sky adds its look direction,
    /// and a pinned time even with the terminator off, since the sky
    /// depends on it.
    fn view_hash(&self) -> String {
        let mut hash = format_view_hash(&self.camera, self.layers_state.wanted(), self.sun_view());
        if self.history.on {
            hash.push_str(&history::format_hash_year(self.history.year));
        }
        if self.paleo.on {
            hash.push_str(&format!("&ma={}", self.paleo.ma));
        }
        if self.langs.on {
            hash.push_str("&languages=1");
        }
        if self.societies.on {
            hash.push_str(&format!("&societies={}", self.societies.variable));
        }
        if self.view == View::Solar {
            if let (false, Some(t)) = (self.sun_on, self.sun_pin) {
                hash.push_str(&format!("&t={t}"));
            }
            hash.push_str(&solar::format_hash(&self.solar_cam));
            if !self.objects_on {
                hash.push_str("&objects=off");
            }
        }
        if self.sky_on {
            if let (false, Some(t)) = (self.sun_on, self.sun_pin) {
                hash.push_str(&format!("&t={t}"));
            }
            hash.push_str(&sky::format_hash(&self.sky_cam));
            if !self.show_figures {
                hash.push_str("&figures=off");
            }
            if self.survey_on {
                hash.push_str("&survey=dss");
            }
        }
        hash
    }

    /// Switch between the Earth, the sky and the solar system.
    fn set_view(&mut self, view: View) {
        self.view = view;
        self.sky_on = view == View::Sky;
        let sky = self.sky_on;
        let doc = self.window.document();
        if let Some(body) = doc.as_ref().and_then(|d| d.body()) {
            let list = body.class_list();
            let _ = list.toggle_with_force("view-sky", sky);
            let _ = list.toggle_with_force("view-solar", view == View::Solar);
            let _ = list.toggle_with_force("view-space", view != View::Earth);
        }
        for (b, v) in self.view_buttons.iter().zip(View::ALL) {
            let pressed = v == view;
            let _ = b.set_attribute("aria-pressed", if pressed { "true" } else { "false" });
        }
        let (hint, labels, aria) = match view {
            View::Sky => (
                "Drag to look around · scroll or pinch to zoom",
                ["Cursor", "Looking", "Field"],
                "Sky view. Drag to look around, scroll to zoom.",
            ),
            View::Solar => (
                "Drag to orbit · scroll or pinch to zoom · tap a planet to fly there",
                ["Cursor", "Orbiting", "Distance"],
                "Solar system. Drag to orbit, scroll to zoom, tap a planet to fly to it.",
            ),
            View::Earth => (
                "Drag to rotate · scroll or pinch to zoom · double-click to fly in",
                ["Cursor", "Center", "Altitude"],
                "Interactive globe. Drag to rotate, scroll to zoom.",
            ),
        };
        self.title_hint.set_text_content(Some(hint));
        for (el, text) in self.hud_labels.iter().zip(labels) {
            el.set_text_content(Some(text));
        }
        let _ = self.canvas.set_attribute("aria-label", aria);
        self.labels.clear();
        self.overlay.clear();
        match view {
            View::Sky => self.credits.show_sky(self.survey_on),
            View::Solar => self.credits.show_solar(),
            View::Earth => self.credits_for_layer(),
        }
        if view == View::Solar {
            self.load_space();
        }
        if view != View::Earth {
            self.load_stars();
            self.send(Event::CancelInertia);
            if self.measure.active {
                self.toggle_measure();
            }
            self.select(None);
            self.set_proj_menu(false);
        }
        if view != View::Sky {
            self.sky_gestures = sky::Gestures::default();
        }
        if view != View::Solar {
            self.solar_gestures = sky::Gestures::default();
            self.solar_press = None;
            self.solar_cam.flight = None;
        }
        let _ = self.canvas.class_list().remove_1("dragging");
        self.player.continuous = view == View::Solar;
        self.playing = self.playing && self.time_shown();
        self.refresh_sun();
        self.redraw.mark_render();
        self.redraw.mark_hud();
        self.hash_stale = true;
    }

    /// Show or hide the deep-sky survey.
    fn set_survey(&mut self, on: bool) {
        self.survey_on = on;
        let _ = self
            .survey_button
            .set_attribute("aria-pressed", if on { "true" } else { "false" });
        if self.sky_on {
            self.credits.show_sky(on);
        }
        if on && self.sky_on {
            self.load_allsky();
        }
        self.redraw.mark_render();
        self.hash_stale = true;
    }

    /// Fetch the survey's Allsky atlas once (trying the mirrors in turn),
    /// and decode and upload it whenever the renderer lacks it.
    fn load_allsky(&mut self) {
        if self.survey_busy || self.renderer.survey_has_allsky() {
            return;
        }
        let Some(app) = self.me.upgrade() else { return };
        self.survey_busy = true;
        let kept = self.survey_allsky.clone();
        let mirror = self.survey_mirror;
        // The small texture tier decodes the atlas at half size (4 MB with
        // mipmaps instead of 17).
        let size = (self.renderer.tile_capacity() < 64).then_some(survey::ALLSKY_SMALL);
        wasm_bindgen_futures::spawn_local(async move {
            let bytes = match kept {
                Some(b) => Ok(b),
                None => {
                    let mut got = Err(JsValue::from_str("no survey mirror answered"));
                    for (i, base) in survey::MIRRORS.iter().enumerate().skip(mirror) {
                        match geo::fetch_bytes(&survey::allsky_url(base)).await {
                            Ok(b) => {
                                app.borrow_mut().survey_mirror = i;
                                got = Ok(Rc::new(b));
                                break;
                            }
                            Err(e) => got = Err(e),
                        }
                    }
                    got
                }
            };
            let bitmap = match &bytes {
                Ok(b) => imagery::decode_sized(b, "image/jpeg", size).await,
                Err(e) => Err(e.clone()),
            };
            let mut a = app.borrow_mut();
            a.survey_busy = false;
            match bitmap {
                Ok(bm) => {
                    if let Err(e) = a.renderer.survey_upload_allsky(&bm) {
                        web_sys::console::error_1(&e);
                    }
                    bm.close();
                    a.survey_allsky = bytes.ok();
                    a.redraw.mark_render();
                }
                Err(e) => {
                    let msg = format!(
                        "deep-sky survey unavailable: {}",
                        e.as_string().unwrap_or_else(|| format!("{e:?}"))
                    );
                    web_sys::console::warn_1(&msg.into());
                }
            }
        });
    }

    /// Fetch, decode and upload one survey tile.
    fn fetch_survey_tile(&self, t: healpix::Tile) {
        let Some(app) = self.me.upgrade() else { return };
        let (epoch, base) = (self.survey.epoch, survey::MIRRORS[self.survey_mirror]);
        let url = survey::tile_url(base, t);
        wasm_bindgen_futures::spawn_local(async move {
            let px = survey::TILE_PX;
            let bitmap = match geo::fetch_bytes(&url).await {
                Ok(b) => imagery::decode_sized(&b, "image/jpeg", Some((px, px))).await,
                Err(e) => Err(e),
            };
            let mut a = app.borrow_mut();
            if a.survey.epoch != epoch {
                a.survey.forget(t);
                return;
            }
            match bitmap {
                Ok(bm) => {
                    let up = a.renderer.survey_upload_tile(t, &bm);
                    bm.close();
                    match up {
                        Ok(()) => {
                            if let Some(old) = a.survey.arrived(t) {
                                a.renderer.survey_drop_tile(old);
                            }
                        }
                        Err(e) => {
                            web_sys::console::error_1(&e);
                            a.survey.failed(t);
                        }
                    }
                }
                Err(_) => a.survey.failed(t), // absent at this order, or offline
            }
            a.redraw.mark_render();
        });
    }

    /// Show or hide the constellation figures.
    fn set_figures(&mut self, on: bool) {
        self.show_figures = on;
        let _ = self
            .figures_button
            .set_attribute("aria-pressed", if on { "true" } else { "false" });
        self.redraw.mark_render();
        self.hash_stale = true;
    }

    /// Fetch and decode the star catalog and the constellation figures, once.
    fn load_stars(&mut self) {
        if std::mem::replace(&mut self.stars_requested, true) {
            return;
        }
        let Some(app) = self.me.upgrade() else { return };
        let figures_app = app.clone();
        let milky_way_app = app.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match data::fetch("milky_way.webp").await {
                Ok(bytes) => {
                    let mut a = milky_way_app.borrow_mut();
                    a.milky_way = Some(Rc::new(bytes));
                    a.decode_milky_way();
                }
                Err(e) => web_sys::console::error_1(&e),
            }
        });
        wasm_bindgen_futures::spawn_local(async move {
            let figs = match data::fetch("constellations.bin").await {
                Ok(bytes) => constellations::decode(&bytes).map_err(JsValue::from),
                Err(e) => Err(e),
            };
            match figs {
                Ok(figs) => {
                    let mut a = figures_app.borrow_mut();
                    a.figures = Some(Rc::new(figs));
                    a.redraw.mark_render();
                    a.set_globe_attr("data-figures", "1");
                }
                Err(e) => web_sys::console::error_1(&e),
            }
        });
        wasm_bindgen_futures::spawn_local(async move {
            let cat = match data::fetch("stars.bin").await {
                Ok(bytes) => stars::decode(&bytes).map_err(JsValue::from),
                Err(e) => Err(e),
            };
            match cat {
                Ok(cat) => {
                    let mut a = app.borrow_mut();
                    a.renderer.set_stars(Rc::new(stars::vertices(&cat)));
                    a.stars = Some(Rc::new(cat));
                    a.redraw.mark_render();
                    // Lets the visual tests wait for the stars.
                    a.set_globe_attr("data-stars", "1");
                }
                Err(e) => web_sys::console::error_1(&e),
            }
        });
    }

    /// The survey's part of a sky frame, starting the fetches it needs;
    /// `None` when it is off or its atlas is not in yet.
    fn plan_survey(
        &mut self,
        cam: &sky::SkyCamera,
        eq_to_hor: &sky::Mat3,
        daylight: f64,
    ) -> Option<sky_render::SurveyFrame> {
        let status = if !self.survey_on {
            "off"
        } else if self.renderer.survey_has_allsky() && self.survey.settled() {
            "settled"
        } else {
            "loading"
        };
        if status != self.survey_attr {
            self.survey_attr = status;
            self.set_globe_attr("data-survey", status);
        }
        if !self.survey_on {
            return None;
        }
        if !self.renderer.survey_has_allsky() {
            self.load_allsky();
            return None;
        }
        let cone = survey::view_cone(cam, eq_to_hor)?;
        let rad_per_px = 2.0 * cam.tan_quarter() / cam.half_short();
        let wanted = survey::wanted_order(rad_per_px)
            .map(|k| survey::Survey::select(k, cone))
            .unwrap_or_default();
        for t in self.survey.plan(wanted) {
            self.fetch_survey_tile(t);
        }
        Some(sky_render::SurveyFrame {
            strength: 1.0 - daylight,
            allsky_indices: survey::allsky_indices(cone),
            tiles: self.survey.drawable(cone),
        })
    }

    /// Decode the Milky Way image and hand it to the renderer (again after a
    /// lost context).
    fn decode_milky_way(&mut self) {
        let (Some(bytes), Some(app)) = (self.milky_way.clone(), self.me.upgrade()) else {
            return;
        };
        if std::mem::replace(&mut self.milky_way_decoding, true) {
            return;
        }
        wasm_bindgen_futures::spawn_local(async move {
            let bitmap = imagery::decode(&bytes, MILKY_WAY_WIDTH).await;
            let mut a = app.borrow_mut();
            a.milky_way_decoding = false;
            match bitmap {
                Ok(b) => {
                    if let Err(e) = a.renderer.upload_milky_way(&b) {
                        web_sys::console::error_1(&e);
                    }
                    b.close();
                    a.redraw.mark_render();
                    a.set_globe_attr("data-milky-way", "1");
                }
                Err(e) => web_sys::console::error_1(&e),
            }
        });
    }

    /// One sky frame: the background and stars, and the labels.
    fn draw_sky(&mut self, pw: u32, ph: u32, dpr: f64) {
        let obs = sky::Observer {
            lat: self.camera.lat,
            lon: self.camera.lon,
            unix_secs: self.sun_time(),
        };
        let cam = self.sky_cam;
        let eq_to_hor = obs.eq_to_horizon();
        let bodies = sky_bodies::bodies(obs.unix_secs, obs.lat, obs.lon);
        let sun_alt = bodies
            .iter()
            .find(|b| b.kind == sky_bodies::Kind::Sun)
            .map_or(-1.0, |b| sky::mul(&eq_to_hor, b.dir)[1].asin());
        let daylight = sky_bodies::daylight(sun_alt);
        let mag_limit =
            stars::limiting_magnitude(cam.fov) - sky_bodies::DAYLIGHT_MAGNITUDES * daylight as f32;
        let star_count = self
            .stars
            .as_ref()
            .map_or(0, |c| stars::count_brighter(c, mag_limit));
        let survey_frame = self.plan_survey(&cam, &eq_to_hor, daylight);
        let frame = sky_render::SkyFrame {
            width: pw,
            height: ph,
            half_short: 0.5 * pw.min(ph) as f64,
            tan_quarter: cam.tan_quarter(),
            cam_to_hor: cam.basis(),
            hor_to_eq: sky::transpose(&eq_to_hor),
            dpr,
            grid_step: sky::grid_step(cam.fov),
            daylight,
            // The survey shows the Milky Way itself.
            milky_way: if survey_frame.is_some() {
                0.0
            } else {
                MILKY_WAY_STRENGTH
            },
            eq_to_cam: sky::matmul(&sky::transpose(&cam.basis()), &eq_to_hor),
            zenith: sky::mul(&sky::transpose(&eq_to_hor), [0.0, 1.0, 0.0]),
            mag_limit,
            star_count,
            survey: survey_frame,
            ground: true,
        };
        if let Err(e) = self.renderer.draw_sky(&frame) {
            web_sys::console::error_1(&e);
        }
        let horizon = sky::horizon_labels(&cam);
        let shown = sky_bodies::on_screen(&bodies, &cam, &eq_to_hor, mag_limit);
        let body_labels = sky_bodies::label_spots(&shown);
        // Bodies and their names come first; star and constellation names
        // keep clear of them.
        let mut taken: Vec<stars::Rect> = shown
            .iter()
            .map(|b| (b.x - b.r, b.y - b.r, b.x + b.r, b.y + b.r))
            .collect();
        taken.extend(shown.iter().zip(&body_labels).filter_map(|(b, s)| {
            let (x, y) = (*s)?;
            Some((x, y - 8.0, x + 7.0 * b.name.chars().count() as f64, y + 8.0))
        }));
        let names = self
            .stars
            .as_ref()
            .map(|c| stars::labels(c, &cam, &eq_to_hor, mag_limit, &taken))
            .unwrap_or_default();
        taken.extend(names.iter().map(|(x, y, n)| stars::label_rect(*x, *y, n)));
        let (lines, figure_names) = match (&self.figures, self.show_figures) {
            (Some(f), true) => (
                constellations::screen_lines(f, &cam, &eq_to_hor),
                constellations::labels(f, &cam, &eq_to_hor, &taken),
            ),
            _ => (Vec::new(), Vec::new()),
        };
        self.overlay.draw_sky(
            &overlay::SkyLabels {
                horizon: &horizon,
                stars: &names,
                lines: &lines,
                figures: &figure_names,
                bodies: &shown,
                body_labels: &body_labels,
                daylight,
            },
            (cam.width, cam.height),
            pw,
            ph,
            dpr,
        );
        if !self.sky_attr && !self.renderer.is_lost() {
            self.sky_attr = true;
            self.set_globe_attr("data-sky", "ready");
        }
        if self.renderer.sky_wants_milky_way() {
            self.decode_milky_way();
        }
    }

    /// The sky view changed: redraw, and rewrite the hash once it settles.
    fn sky_moved(&mut self) {
        self.redraw.mark_render();
        self.redraw.mark_hud();
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
        if self.history.on || self.paleo.on {
            return None; // the IDs on the GPU are the year's polities
        }
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
        if self.view == View::Solar {
            self.solar_frame(cw, ch, pw, ph, dpr, dt, now_ms);
            return;
        }
        if self.sky_on {
            self.sky_cam.width = cw;
            self.sky_cam.height = ch;
            let (draw, hud) = self.redraw.take();
            if draw {
                self.draw_sky(pw, ph, dpr);
                self.hash_stale = true;
            }
            if hud {
                self.update_hud();
            }
            let scrubbing = now_ms - self.last_scrub_ms < SCRUB_HASH_DELAY_MS;
            if self.hash_stale && !self.playing && !self.sky_gestures.active() && !scrubbing {
                self.write_hash();
            }
            return;
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
        self.history_frame();
        self.paleo_frame();
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
                let marks = self.language_marks().or_else(|| self.society_marks());
                self.overlay
                    .draw(&self.camera, &self.measure, marks.as_ref(), pw, ph, dpr);
            }
            self.hash_stale = true;
        }
        if hud {
            self.update_hud();
        }
        // A slider drag changes the pin on every input event: wait until it pauses.
        let scrubbing = now_ms - self.last_scrub_ms < SCRUB_HASH_DELAY_MS;
        if self.hash_stale && !animating && !self.controls.dragging() && !scrubbing {
            self.write_hash();
        }
    }

    /// Replace the URL hash with the current view, if it changed.
    fn write_hash(&mut self) {
        self.hash_stale = false;
        let hash = self.view_hash();
        if hash != self.last_hash {
            if let Ok(history) = self.window.history() {
                let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&hash));
            }
            self.last_hash = hash;
        }
    }

    fn update_hud(&mut self) {
        if self.view == View::Solar {
            return self.solar_hud();
        }
        if self.sky_on {
            let to_eq = sky::transpose(
                &sky::Observer {
                    lat: self.camera.lat,
                    lon: self.camera.lon,
                    unix_secs: self.sun_time(),
                }
                .eq_to_horizon(),
            );
            let cursor = self
                .cursor
                .and_then(|(x, y)| self.sky_cam.pick(x, y))
                .map(|h| {
                    let (ra, dec) = sky::vec_radec(sky::mul(&to_eq, h));
                    sky::format_radec(ra, dec)
                })
                .unwrap_or_else(|| "—".into());
            self.hud_cursor.set_text_content(Some(&cursor));
            self.hud_center
                .set_text_content(Some(&sky::format_look(&self.sky_cam)));
            self.hud_alt
                .set_text_content(Some(&format!("{:.1}°", self.sky_cam.fov.to_degrees())));
            return;
        }
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
        let polity = self.history.on.then(|| {
            let (lat, lon) = self.cursor.and_then(|(x, y)| self.camera.pick(x, y))?;
            self.history_at(lat.to_degrees(), lon.to_degrees())
        });
        let language = self
            .cursor
            .and_then(|(x, y)| self.language_at(x, y).or_else(|| self.society_at(x, y)));
        let country = match &polity {
            _ if language.is_some() => language.as_deref().unwrap_or("—"),
            Some(p) => p.as_deref().unwrap_or("—"),
            None => self
                .hover
                .and_then(|i| self.countries.as_ref()?.get(i))
                .map_or("—", |c| c.name.as_str()),
        };
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
        if self.view == View::Solar {
            return self.solar_pointer_down(e);
        }
        if self.sky_on {
            let _ = self.canvas.set_pointer_capture(e.pointer_id());
            self.sky_gestures
                .down(e.pointer_id(), e.client_x() as f64, e.client_y() as f64);
            let _ = self.canvas.class_list().add_1("dragging");
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
        if self.view == View::Solar {
            return self.solar_pointer_move(e.pointer_id(), x, y);
        }
        if self.sky_on {
            if self
                .sky_gestures
                .moved(&mut self.sky_cam, e.pointer_id(), x, y)
            {
                self.sky_moved();
            }
            return;
        }
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
        if self.view == View::Solar {
            return self.solar_pointer_up(e);
        }
        if self.sky_on {
            self.sky_gestures.up(e.pointer_id());
            if !self.sky_gestures.active() {
                let _ = self.canvas.class_list().remove_1("dragging");
                self.hash_stale = true;
            }
            return;
        }
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
    let initial_hash = window.location().hash().unwrap_or_default();
    let initial_view = View::from_hash(&initial_hash);
    let initial_look = sky::parse_hash_look(&initial_hash);
    let initial_orbit = solar::parse_hash_orbit(&initial_hash);
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
        sky_on: false,
        sky_cam: initial_look.unwrap_or_default(),
        sky_gestures: sky::Gestures::default(),
        view: View::Earth,
        solar_cam: initial_orbit.unwrap_or_default(),
        solar_gestures: sky::Gestures::default(),
        solar_press: None,
        solar_drawn: Vec::new(),
        solar_labels: Vec::new(),
        solar_select: element(&doc, "solar-target")?,
        space: None,
        space_requested: false,
        objects_on: !initial_hash.contains("&objects=off"),
        objects_button: element(&doc, "objects-toggle")?,
        solar_markers: Vec::new(),
        planet_bytes: HashMap::new(),
        planet_busy: Default::default(),
        planet_failed: Default::default(),
        history: history_app::HistoryState::new(history::parse_hash_year(&initial_hash)),
        year_ui: history_app::YearUi::new(&doc)?,
        paleo: paleo_app::PaleoState::new(paleo::parse_hash_ma(&initial_hash)),
        ages_ui: paleo_app::AgesUi::new(&doc)?,
        langs: languages_app::LanguagesState::default(),
        langs_button: element(&doc, "languages-toggle")?,
        search: None,
        societies: ethnography_app::SocietiesState::default(),
        societies_button: element(&doc, "societies-toggle")?,
        ethno_select: element(&doc, "ethno-var")?,
        view_buttons: [
            element(&doc, "view-earth")?,
            element(&doc, "view-sky")?,
            element(&doc, "view-solar")?,
        ],
        title_hint: element(&doc, "title-hint")?,
        hud_labels: [
            element(&doc, "hud-l-cursor")?,
            element(&doc, "hud-l-center")?,
            element(&doc, "hud-l-alt")?,
        ],
        sky_attr: false,
        stars: None,
        stars_requested: false,
        figures: None,
        show_figures: !initial_hash.contains("&figures=off"),
        figures_button: element(&doc, "figures-toggle")?,
        milky_way: None,
        milky_way_decoding: false,
        survey_on: initial_hash.contains("&survey=dss"),
        survey: survey::Survey::new(renderer_tile_capacity),
        survey_allsky: None,
        survey_busy: false,
        survey_mirror: 0,
        survey_button: element(&doc, "survey-toggle")?,
        survey_attr: "",
    }));

    {
        let mut a = app.borrow_mut();
        a.me = Rc::downgrade(&app);
        // The URL layer takes the same path as a click.
        a.select_layer(initial_layer);
        let figures = a.show_figures;
        a.set_figures(figures); // labels the button
        let objects = a.objects_on;
        a.set_objects(objects);
        let survey = a.survey_on;
        a.set_survey(survey);
        if initial_view != View::Earth {
            a.set_view(initial_view);
        }
        if a.history.on {
            a.history.on = false; // set_history acts on a change
            a.set_history(true);
        }
        if a.paleo.on {
            a.paleo.on = false; // set_paleo acts on a change
            a.set_paleo(true);
        }
        if languages_app::parse_hash(&initial_hash) {
            a.set_languages(true);
        }
        if let Some(v) = ethnography_app::parse_hash(&initial_hash) {
            a.societies.variable = v;
            a.set_societies(true);
        }
        a.sync_solar_select();
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
            if a.time_shown() && a.sun_pin.is_none() {
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
            let mut a = app.borrow_mut();
            if a.view == View::Solar {
                let f = sky::wheel_factor(e.delta_y(), e.delta_mode(), e.ctrl_key());
                a.solar_cam.zoom_by(f);
                a.solar_moved();
                return;
            }
            if a.sky_on {
                let f = sky::wheel_factor(e.delta_y(), e.delta_mode(), e.ctrl_key());
                let at = (e.client_x() as f64, e.client_y() as f64);
                a.sky_cam.zoom(f, Some(at));
                a.sky_moved();
                return;
            }
            a.send(Event::Wheel {
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
            if a.view == View::Solar {
                return; // the first tap already flew to what is under it
            }
            if a.sky_on {
                let factor = if e.shift_key() { 2.0 } else { 0.5 };
                let at = (e.client_x() as f64, e.client_y() as f64);
                a.sky_cam.zoom(factor, Some(at));
                a.sky_moved();
                return;
            }
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
                        || t.is_instance_of::<web_sys::HtmlSelectElement>()
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
                let plain = !(e.ctrl_key() || e.meta_key() || e.alt_key());
                if plain && (e.key() == "v" || e.key() == "V") {
                    let next = if e.shift_key() {
                        a.view.prev()
                    } else {
                        a.view.next()
                    };
                    a.set_view(next);
                    return;
                }
                if a.view == View::Solar {
                    if plain && (e.key() == "s" || e.key() == "S") {
                        let (ui, url) = (a.share_ui.clone(), a.current_share_url());
                        drop(a);
                        share::share(&ui, url);
                        return;
                    }
                    if a.solar_key(&e, plain) {
                        e.prevent_default();
                    }
                    return;
                }
                if a.sky_on {
                    let cam = &mut a.sky_cam;
                    match e.key().as_str() {
                        "ArrowLeft" => cam.nudge(-1.0, 0.0),
                        "ArrowRight" => cam.nudge(1.0, 0.0),
                        "ArrowUp" => cam.nudge(0.0, 1.0),
                        "ArrowDown" => cam.nudge(0.0, -1.0),
                        "+" | "=" => cam.zoom(0.6, None),
                        "-" | "_" => cam.zoom(1.0 / 0.6, None),
                        "r" | "R" if plain => {
                            *cam = sky::SkyCamera {
                                width: cam.width,
                                height: cam.height,
                                ..sky::SkyCamera::default()
                            }
                        }
                        _ => {
                            // Keys the sky shares with the Earth view, and C.
                            match e.key().as_str() {
                                "c" | "C" if plain => {
                                    let on = !a.show_figures;
                                    a.set_figures(on);
                                }
                                "d" | "D" if plain => {
                                    let on = !a.survey_on;
                                    a.set_survey(on);
                                }
                                "s" | "S" if plain => {
                                    let (ui, url) = (a.share_ui.clone(), a.current_share_url());
                                    drop(a);
                                    share::share(&ui, url);
                                }
                                "," | "." | "<" | ">" if plain => {
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
                                "t" | "T" if plain => {
                                    let playing = a.playing;
                                    a.set_playing(!playing);
                                }
                                "`" => {
                                    a.renderer.debug_toggle_context_loss();
                                }
                                _ => {}
                            }
                            return;
                        }
                    };
                    e.prevent_default();
                    a.hash_stale = true;
                    a.sky_moved();
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
                    "n" | "N"
                        if !(e.ctrl_key() || e.meta_key() || e.alt_key()) && !a.history.on =>
                    {
                        a.toggle_sun()
                    }
                    "k" | "K" if !(e.ctrl_key() || e.meta_key() || e.alt_key()) => {
                        let on = !a.societies.on;
                        a.set_societies(on);
                    }
                    "w" | "W" if !(e.ctrl_key() || e.meta_key() || e.alt_key()) => {
                        let on = !a.langs.on;
                        a.set_languages(on);
                    }
                    "a" | "A" if !(e.ctrl_key() || e.meta_key() || e.alt_key()) => {
                        let on = !a.paleo.on;
                        a.set_paleo(on);
                    }
                    // Deep time: , and . step a million years, Shift (< and
                    // >) ten.
                    k @ ("," | "." | "<" | ">")
                        if a.paleo.on && !(e.ctrl_key() || e.meta_key() || e.alt_key()) =>
                    {
                        let shift = e.shift_key();
                        a.paleo_key(k, shift);
                    }
                    "h" | "H" if !(e.ctrl_key() || e.meta_key() || e.alt_key()) => {
                        let on = !a.history.on;
                        a.set_history(on);
                    }
                    // Historical borders: , and . step a year, Shift (< and >)
                    // a century.
                    "," | "." | "<" | ">"
                        if a.history.on && !(e.ctrl_key() || e.meta_key() || e.alt_key()) =>
                    {
                        let key = e.key();
                        let step = if e.shift_key() || key == "<" || key == ">" {
                            100
                        } else {
                            1
                        };
                        a.step_year(if key == "," || key == "<" {
                            -step
                        } else {
                            step
                        });
                    }
                    // Time scrubber (terminator on): , and . step an hour back and
                    // forward, Shift (< and >) a day; T plays and pauses.
                    "," | "." | "<" | ">"
                        if a.time_shown() && !(e.ctrl_key() || e.meta_key() || e.alt_key()) =>
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
                    "t" | "T"
                        if a.time_shown() && !(e.ctrl_key() || e.meta_key() || e.alt_key()) =>
                    {
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
                let view = View::from_hash(&hash);
                let year = history::parse_hash_year(&hash);
                if let Some(y) = year {
                    a.set_year(y);
                }
                if year.is_some() != a.history.on {
                    a.set_history(year.is_some());
                }
                let ma = paleo::parse_hash_ma(&hash);
                if let Some(m) = ma {
                    a.set_ma(m);
                }
                if ma.is_some() != a.paleo.on {
                    a.set_paleo(ma.is_some());
                }
                let langs = languages_app::parse_hash(&hash);
                if langs != a.langs.on {
                    a.set_languages(langs);
                }
                let societies = ethnography_app::parse_hash(&hash);
                if let Some(v) = &societies {
                    a.set_society_variable(v);
                }
                if societies.is_some() != a.societies.on {
                    a.set_societies(societies.is_some());
                }
                if let Some(cam) = solar::parse_hash_orbit(&hash) {
                    let (w, h) = (a.solar_cam.look.width, a.solar_cam.look.height);
                    a.solar_cam = cam;
                    a.solar_cam.look.width = w;
                    a.solar_cam.look.height = h;
                }
                if let Some(look) = sky::parse_hash_look(&hash) {
                    let (w, h) = (a.sky_cam.width, a.sky_cam.height);
                    a.sky_cam = sky::SkyCamera {
                        width: w,
                        height: h,
                        ..look
                    };
                }
                a.set_figures(!hash.contains("&figures=off"));
                a.objects_on = !hash.contains("&objects=off");
                a.set_survey(hash.contains("&survey=dss"));
                if view != a.view {
                    a.set_view(view);
                } else {
                    a.sky_moved();
                    a.sync_solar_select();
                }
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
    let buttons: [(&str, Action); 18] = [
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
            if a.view == View::Solar {
                a.solar_cam.zoom_by(0.5);
                return a.solar_moved();
            }
            if a.sky_on {
                a.sky_cam.zoom(0.5, None);
                a.hash_stale = true;
                return a.sky_moved();
            }
            a.send(Event::Zoom {
                factor: 0.5,
                toward: None,
            });
        }),
        ("zoom-out", |a| {
            if a.view == View::Solar {
                a.solar_cam.zoom_by(2.0);
                return a.solar_moved();
            }
            if a.sky_on {
                a.sky_cam.zoom(2.0, None);
                a.hash_stale = true;
                return a.sky_moved();
            }
            a.send(Event::Zoom {
                factor: 2.0,
                toward: None,
            });
        }),
        ("reset", |a| {
            if a.view == View::Solar {
                return a.solar_reset();
            }
            if a.sky_on {
                let (width, height) = (a.sky_cam.width, a.sky_cam.height);
                a.sky_cam = sky::SkyCamera {
                    width,
                    height,
                    ..sky::SkyCamera::default()
                };
                a.hash_stale = true;
                return a.sky_moved();
            }
            a.send(Event::Reset);
        }),
        ("survey-toggle", |a| {
            let on = !a.survey_on;
            a.set_survey(on)
        }),
        ("figures-toggle", |a| {
            let on = !a.show_figures;
            a.set_figures(on)
        }),
        ("view-earth", |a| a.set_view(View::Earth)),
        ("view-sky", |a| a.set_view(View::Sky)),
        ("view-solar", |a| a.set_view(View::Solar)),
        ("societies-toggle", |a| {
            let on = !a.societies.on;
            a.set_societies(on)
        }),
        ("languages-toggle", |a| {
            let on = !a.langs.on;
            a.set_languages(on)
        }),
        ("paleo-toggle", |a| {
            let on = !a.paleo.on;
            a.set_paleo(on)
        }),
        ("history-toggle", |a| {
            let on = !a.history.on;
            a.set_history(on)
        }),
        ("objects-toggle", |a| {
            let on = !a.objects_on;
            a.set_objects(on)
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

    solar_app::wire(&app)?;
    history_app::wire(&app, &doc)?;
    ethnography_app::wire(&app, &doc)?;
    paleo_app::wire(&app, &doc)?;

    // Place search: fly to the chosen place.
    {
        let app2 = app.clone();
        let search = search_ui::init(&doc, move |lat, lon, alt| {
            app2.borrow_mut().send(Event::FlyTo {
                lat,
                lon,
                alt,
                bearing: None,
            });
        })?;
        app.borrow_mut().search = Some(search);
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
                    a.history.modern_labels = Some(list.clone());
                    if a.history.on || a.paleo.on {
                        a.history_reapply();
                        a.paleo_reapply();
                    } else {
                        a.labels.set_labels(list);
                    }
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
                    if a.history.on {
                        a.history_reapply(); // the year's IDs go back on top
                    }
                    if a.paleo.on {
                        a.paleo_reapply();
                    }
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
