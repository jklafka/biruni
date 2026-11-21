//! The solar system view's part of the app: its frames, pointer and key
//! input, HUD, and the target menu. The math is in `solar.rs` and
//! `solar_scene.rs` (browser-free); this is the glue, as `main.rs` is for
//! the Earth and sky views.

use crate::solar::{self, Body, SOLAR_TO_EQ};
use crate::solar_render::BodyFrame;
use crate::{
    ephemeris, scrub, sky, sky_render, solar_scene, stars, App, MILKY_WAY_STRENGTH,
    SCRUB_HASH_DELAY_MS,
};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{KeyboardEvent, PointerEvent};

/// A press that moves less than this (CSS pixels) is a tap.
const TAP_SLOP: f64 = 6.0;

impl App {
    /// The scrubber's time as TT centuries, which `solar` works in.
    fn solar_t(&self) -> Option<f64> {
        ephemeris::centuries_tt_from_unix(self.sun_time())
    }

    fn reduced_motion(&self) -> bool {
        self.window
            .match_media("(prefers-reduced-motion: reduce)")
            .ok()
            .flatten()
            .is_some_and(|m| m.matches())
    }

    /// One animation frame in the solar system view.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn solar_frame(
        &mut self,
        cw: f64,
        ch: f64,
        pw: u32,
        ph: u32,
        dpr: f64,
        dt: f64,
        now_ms: f64,
    ) {
        self.solar_cam.look.width = cw;
        self.solar_cam.look.height = ch;
        if self.solar_cam.flight.is_some() {
            // Also the frame that lands, once the flight has ended.
            self.solar_cam.step(dt);
            self.redraw.mark_render();
            self.redraw.mark_hud();
        }
        let (draw, hud) = self.redraw.take();
        if draw {
            self.draw_solar(pw, ph, dpr);
            self.hash_stale = true;
        }
        if hud {
            self.update_hud();
        }
        let scrubbing = now_ms - self.last_scrub_ms < SCRUB_HASH_DELAY_MS;
        let busy = self.playing || self.solar_cam.flight.is_some() || self.solar_gestures.active();
        if self.hash_stale && !busy && !scrubbing {
            self.write_hash();
        }
    }

    fn draw_solar(&mut self, pw: u32, ph: u32, dpr: f64) {
        let cam = self.solar_cam;
        let Some((t, eye)) = self.solar_t().and_then(|t| Some((t, cam.eye(t)?))) else {
            // Outside the ephemerides (the scrubber cannot get there).
            return;
        };
        let basis = cam.look.basis();
        let mag_limit = stars::limiting_magnitude(cam.look.fov);
        let star_count = self
            .stars
            .as_ref()
            .map_or(0, |c| stars::count_brighter(c, mag_limit));
        let half_short = 0.5 * pw.min(ph) as f64;
        let frame = sky_render::SkyFrame {
            width: pw,
            height: ph,
            half_short,
            tan_quarter: cam.look.tan_quarter(),
            cam_to_hor: basis,
            hor_to_eq: SOLAR_TO_EQ,
            dpr,
            grid_step: 0.0,
            daylight: 0.0,
            milky_way: MILKY_WAY_STRENGTH,
            eq_to_cam: sky::matmul(&sky::transpose(&basis), &sky::transpose(&SOLAR_TO_EQ)),
            zenith: [0.0, 1.0, 0.0],
            mag_limit,
            star_count,
            survey: None,
            ground: false,
        };
        let drawn = solar_scene::drawn(&cam, eye, t);
        let bodies = BodyFrame {
            width: pw,
            height: ph,
            half_short,
            tan_quarter: cam.look.tan_quarter(),
            dpr,
        };
        if let Err(e) = self.renderer.draw_solar(&frame, &bodies, &drawn) {
            web_sys::console::error_1(&e);
        }
        let orbits = solar_scene::orbits(&cam, eye, t, &solar_scene::occluders(&drawn));
        let labels = solar_scene::labels(&cam, &drawn);
        self.overlay.draw_solar(
            &orbits,
            &labels,
            cam.target,
            (cam.look.width, cam.look.height),
            pw,
            ph,
            dpr,
        );
        self.solar_drawn = drawn;
        self.solar_labels = labels;
        if !self.renderer.is_lost() {
            self.set_globe_attr("data-solar", "ready");
        }
        if self.renderer.sky_wants_milky_way() {
            self.decode_milky_way();
        }
    }

    pub(crate) fn solar_hud(&mut self) {
        let t = self.solar_t();
        let eye = t.and_then(|t| self.solar_cam.eye(t));
        let cursor = self.cursor.and_then(|(x, y)| {
            let (t, eye) = (t?, eye?);
            match solar_scene::hit(&self.solar_drawn, &self.solar_labels, x, y) {
                Some(b) => solar_scene::describe(b, eye, t),
                None => {
                    let d = self.solar_cam.look.pick(x, y)?;
                    let (ra, dec) = sky::vec_radec(sky::mul(&SOLAR_TO_EQ, d));
                    Some(sky::format_radec(ra, dec))
                }
            }
        });
        self.hud_cursor
            .set_text_content(Some(cursor.as_deref().unwrap_or("—")));
        let target = self.solar_cam.target;
        self.hud_center.set_text_content(Some(target.name()));
        let above = t.and_then(|t| self.solar_cam.focus(t)).map(|(_, d)| {
            d - if self.solar_cam.flight.is_some() {
                0.0
            } else {
                target.radius_au()
            }
        });
        let text = above.map_or_else(|| "—".into(), |d| solar::format_distance(d.max(0.0)));
        self.hud_alt.set_text_content(Some(&text));
    }

    /// The solar camera changed: redraw, and rewrite the hash once it
    /// settles.
    pub(crate) fn solar_moved(&mut self) {
        self.redraw.mark_render();
        self.redraw.mark_hud();
        self.hash_stale = true;
    }

    pub(crate) fn fly_solar(&mut self, to: Body) {
        let Some(t) = self.solar_t() else { return };
        let reduced = self.reduced_motion();
        self.solar_cam.fly_to(to, t, reduced);
        self.sync_solar_select();
        self.solar_moved();
    }

    /// Back to the home view: the Sun from above the inner planets.
    pub(crate) fn solar_reset(&mut self) {
        let (_, az, alt, _) = solar::HOME;
        let fov = self.solar_cam.look.fov;
        self.solar_cam.look.set(az, alt, fov);
        self.fly_solar(Body::Sun);
    }

    /// Show the target in the menu.
    pub(crate) fn sync_solar_select(&self) {
        self.solar_select.set_value(&self.solar_cam.target.key());
    }

    pub(crate) fn solar_pointer_down(&mut self, e: &PointerEvent) {
        let (x, y) = (e.client_x() as f64, e.client_y() as f64);
        let _ = self.canvas.set_pointer_capture(e.pointer_id());
        let first = !self.solar_gestures.active();
        self.solar_gestures.down(e.pointer_id(), x, y);
        self.solar_press = first.then_some((x, y));
        let _ = self.canvas.class_list().add_1("dragging");
    }

    pub(crate) fn solar_pointer_move(&mut self, id: i32, x: f64, y: f64) {
        if let Some((px, py)) = self.solar_press {
            if (x - px).hypot(y - py) > TAP_SLOP {
                self.solar_press = None;
            }
        }
        if self.solar_gestures.moved(&mut self.solar_cam, id, x, y) {
            self.solar_moved();
        }
    }

    pub(crate) fn solar_pointer_up(&mut self, e: &PointerEvent) {
        let tap = self.solar_press.take().filter(|_| e.type_() == "pointerup");
        let was_active = self.solar_gestures.active();
        self.solar_gestures.up(e.pointer_id());
        if let (Some((x, y)), true) = (tap, was_active) {
            if let Some(b) = solar_scene::hit(&self.solar_drawn, &self.solar_labels, x, y) {
                self.fly_solar(b);
            }
        }
        if !self.solar_gestures.active() {
            let _ = self.canvas.class_list().remove_1("dragging");
            self.hash_stale = true;
        }
    }

    /// Keys of the solar system view; returns whether the key was used.
    pub(crate) fn solar_key(&mut self, e: &KeyboardEvent, plain: bool) -> bool {
        let key = e.key();
        let cam = &mut self.solar_cam;
        match key.as_str() {
            "ArrowLeft" => cam.nudge(-1.0, 0.0),
            "ArrowRight" => cam.nudge(1.0, 0.0),
            "ArrowUp" => cam.nudge(0.0, -1.0),
            "ArrowDown" => cam.nudge(0.0, 1.0),
            "+" | "=" => cam.zoom_by(0.6),
            "-" | "_" => cam.zoom_by(1.0 / 0.6),
            "r" | "R" if plain => {
                self.solar_reset();
                return true;
            }
            k if plain && k.len() == 1 && k.as_bytes()[0].is_ascii_digit() => {
                let i = (k.as_bytes()[0] - b'0') as usize;
                self.fly_solar(DIGIT_BODIES[i]);
                return true;
            }
            "," | "." | "<" | ">" if plain => {
                let step = if e.shift_key() || key == "<" || key == ">" {
                    scrub::DAY
                } else {
                    scrub::HOUR
                };
                self.scrub_by(if key == "," || key == "<" {
                    -step
                } else {
                    step
                });
                return true;
            }
            "t" | "T" if plain => {
                let playing = self.playing;
                self.set_playing(!playing);
                return true;
            }
            "`" => {
                self.renderer.debug_toggle_context_loss();
                return true;
            }
            _ => return false,
        }
        self.solar_moved();
        true
    }
}

/// Number keys: the planets by their order from the Sun, 0 the Sun and 9
/// the Moon.
pub const DIGIT_BODIES: [Body; 10] = [
    Body::Sun,
    Body::Mercury,
    Body::Venus,
    Body::Earth,
    Body::Mars,
    Body::Jupiter,
    Body::Saturn,
    Body::Uranus,
    Body::Neptune,
    Body::Moon,
];

/// The target menu flies to the chosen body.
pub fn wire(app: &Rc<RefCell<App>>) -> Result<(), JsValue> {
    let select = app.borrow().solar_select.clone();
    let app = app.clone();
    crate::listen(select.as_ref(), "change", true, move |e: web_sys::Event| {
        let Some(sel) = e
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlSelectElement>().ok())
        else {
            return;
        };
        if let Some(b) = Body::from_key(&sel.value()) {
            app.borrow_mut().fly_solar(b);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_name_every_body_once() {
        for b in Body::ALL {
            assert_eq!(DIGIT_BODIES.iter().filter(|d| **d == b).count(), 1, "{b:?}");
        }
        assert_eq!(DIGIT_BODIES[3], Body::Earth);
        assert_eq!(DIGIT_BODIES[8], Body::Neptune);
    }
}
