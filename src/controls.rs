//! Browser-free input state machine.
//!
//! `main.rs` translates DOM events into [`Event`]s and feeds them to
//! [`Controls::handle`], which mutates a [`Camera`]. Drag, pinch, fling
//! inertia, wheel and button zoom, zoom anchoring, fly-to and reset all live
//! here, so they can be unit tested natively. So do the rotation inputs:
//! two-finger twist, alt-drag and the keyboard step turn the view (and
//! unlock north-up), and the compass restores north-up with a flight.
//!
//! Event times (`t`, milliseconds) come from the caller's clock. Events with
//! a non-finite number in them are ignored, so garbage input can never poison
//! the camera.

use crate::camera::{wrap_angle, Camera, MAX_ALT, MIN_ALT};
use crate::view::HOME;
use std::f64::consts::PI;

/// A press that moves less than this many CSS pixels is a tap, not a drag.
pub const TAP_SLOP_PX: f64 = 6.0;
/// Arrow-key impulse in CSS pixels per millisecond; decays via inertia.
pub const NUDGE_SPEED: f64 = 0.6;
/// A pause this long (ms) between the last move and release cancels the fling.
const FLING_PAUSE_MS: f64 = 80.0;
/// Longest step a single tick may integrate, in milliseconds.
const MAX_DT_MS: f64 = 100.0;
/// A two-finger twist is ignored until it adds up to this many radians, so
/// an ordinary pinch never unlocks north-up by accident (about 7 degrees).
const TWIST_START: f64 = 0.12;
/// Within this many pixels of the screen center, alt-drag turns less, since
/// the angle swept around the center changes wildly there.
const ROTATE_DEADZONE_PX: f64 = 40.0;
/// Keyboard rotation step, radians (15 degrees).
pub const ROTATE_STEP: f64 = PI / 12.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    PointerDown {
        id: i32,
        x: f64,
        y: f64,
        t: f64,
    },
    /// `alt` is the Alt/Option key: a single-pointer drag then rotates the
    /// view about the screen center instead of panning.
    PointerMove {
        id: i32,
        x: f64,
        y: f64,
        t: f64,
        alt: bool,
    },
    /// A release. `cancel` is a `pointercancel`, which never counts as a tap.
    PointerUp {
        id: i32,
        t: f64,
        cancel: bool,
    },
    /// `mode` is the DOM `deltaMode`: 0 pixels, 1 lines, 2 pages.
    Wheel {
        dy: f64,
        x: f64,
        y: f64,
        mode: u32,
        ctrl: bool,
    },
    /// Advance animations by `dt` milliseconds.
    Tick {
        dt: f64,
    },
    /// Zoom by a factor on altitude, optionally toward a screen point.
    Zoom {
        factor: f64,
        toward: Option<(f64, f64)>,
    },
    /// Double-click: fly to the point under the cursor, closer or (shift) farther.
    DoubleClick {
        x: f64,
        y: f64,
        shift: bool,
    },
    /// Arrow key: give the view an inertial push.
    Nudge(Direction),
    /// `bearing`: `Some` sets the view bearing (and north-up lock when it is
    /// 0); `None` keeps it, or restores north when the lock is on.
    FlyTo {
        lat: f64,
        lon: f64,
        alt: f64,
        bearing: Option<f64>,
    },
    /// The compass: animate back to north-up and lock it.
    NorthUp,
    /// Toggle the north-up lock; locking animates to north-up.
    ToggleNorthUp,
    /// Turn the view by this many radians (positive is clockwise) with a
    /// short animation. Unlocks north-up.
    Rotate(f64),
    Reset,
    /// Drop pan inertia only (e.g. on context loss).
    CancelInertia,
}

/// What an event did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Response {
    /// The camera moved or an animation was started: redraw the globe.
    pub render: bool,
    /// A lone press was released in place: a click at these screen coordinates.
    pub tap: Option<(f64, f64)>,
}

struct Pointer {
    id: i32,
    x: f64,
    y: f64,
}

/// A lone press that has not moved past `TAP_SLOP_PX` yet.
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
    bearing: f64,
}

pub struct Controls {
    target_alt: f64,
    anchor: Option<Anchor>,
    flight: Option<Flight>,
    pointers: Vec<Pointer>,
    /// Set once a second finger has moved; suppresses the fling.
    pinch: bool,
    /// Pan inertia in CSS pixels per millisecond.
    velocity: (f64, f64),
    last_move_ms: f64,
    press: Option<Press>,
    /// Twist accumulated by the current two-finger gesture, radians.
    twist_acc: f64,
    /// The twist has passed `TWIST_START`, so it now turns the view.
    twisting: bool,
}

fn finite(vals: &[f64]) -> bool {
    vals.iter().all(|v| v.is_finite())
}

impl Event {
    fn is_finite(&self) -> bool {
        match *self {
            Event::PointerDown { x, y, t, .. } | Event::PointerMove { x, y, t, .. } => {
                finite(&[x, y, t])
            }
            Event::PointerUp { t, .. } => finite(&[t]),
            Event::Wheel { dy, x, y, .. } => finite(&[dy, x, y]),
            Event::Tick { dt } => finite(&[dt]),
            Event::Zoom { factor, toward } => {
                factor.is_finite() && factor > 0.0 && toward.is_none_or(|(x, y)| finite(&[x, y]))
            }
            Event::DoubleClick { x, y, .. } => finite(&[x, y]),
            Event::FlyTo {
                lat,
                lon,
                alt,
                bearing,
            } => finite(&[lat, lon, alt]) && bearing.is_none_or(f64::is_finite),
            Event::Rotate(a) => finite(&[a]),
            Event::Nudge(_)
            | Event::NorthUp
            | Event::ToggleNorthUp
            | Event::Reset
            | Event::CancelInertia => true,
        }
    }
}

impl Controls {
    pub fn new(camera: &Camera) -> Self {
        Self {
            target_alt: camera.alt,
            anchor: None,
            flight: None,
            pointers: Vec::new(),
            pinch: false,
            velocity: (0.0, 0.0),
            last_move_ms: 0.0,
            press: None,
            twist_acc: 0.0,
            twisting: false,
        }
    }

    /// A pointer is down (a drag or pinch is in progress).
    pub fn dragging(&self) -> bool {
        !self.pointers.is_empty()
    }

    /// True while the view is changing and a lower-quality frame is
    /// acceptable: a drag or pinch past the tap slop, a flight, a fling, or a
    /// zoom still easing toward its target. A press that has not moved yet
    /// (a click in the making) does not count.
    pub fn in_motion(&self, cam: &Camera) -> bool {
        let zooming = (self.target_alt.ln() - cam.alt.ln()).abs() > 1e-4;
        (self.dragging() && self.press.is_none())
            || self.flight.is_some()
            || self.velocity != (0.0, 0.0)
            || zooming
    }

    /// Whether the view is moving on its own (flight or fling).
    #[cfg(test)]
    fn animating(&self) -> bool {
        self.flight.is_some() || self.velocity != (0.0, 0.0)
    }

    pub fn handle(&mut self, cam: &mut Camera, ev: Event) -> Response {
        let mut out = Response::default();
        if !ev.is_finite() {
            return out;
        }
        match ev {
            Event::PointerDown { id, x, y, t } => self.pointer_down(cam, id, x, y, t),
            Event::PointerMove { id, x, y, t, alt } => {
                out.render = self.pointer_move(cam, id, (x, y), t, alt)
            }
            Event::PointerUp { id, t, cancel } => {
                out.tap = self.pointer_up(id, t, cancel);
                out.render = true;
            }
            Event::Wheel {
                dy,
                x,
                y,
                mode,
                ctrl,
            } => {
                let scale = match mode {
                    1 => 16.0,  // lines
                    2 => 400.0, // pages
                    _ => 1.0,
                };
                // Trackpad pinch arrives as ctrl+wheel with small deltas.
                let k = if ctrl { 0.01 } else { 0.0018 };
                let factor = (dy * scale * k).clamp(-1.0, 1.0).exp();
                self.zoom(cam, factor, Some((x, y)));
                out.render = true;
            }
            Event::Tick { dt } => out.render = self.tick(cam, dt),
            Event::Zoom { factor, toward } => {
                self.zoom(cam, factor, toward);
                out.render = true;
            }
            Event::DoubleClick { x, y, shift } => {
                if let Some((lat, lon)) = cam.pick(x, y) {
                    let alt = cam.alt * if shift { 2.5 } else { 0.4 };
                    self.fly_to(cam, lat, lon, alt, None);
                    out.render = true;
                }
            }
            Event::Nudge(d) => {
                match d {
                    Direction::Left => self.velocity.0 = NUDGE_SPEED,
                    Direction::Right => self.velocity.0 = -NUDGE_SPEED,
                    Direction::Up => self.velocity.1 = NUDGE_SPEED,
                    Direction::Down => self.velocity.1 = -NUDGE_SPEED,
                }
                out.render = true;
            }
            Event::FlyTo {
                lat,
                lon,
                alt,
                bearing,
            } => {
                if let Some(b) = bearing {
                    cam.north_up = wrap_angle(b) == 0.0;
                }
                self.fly_to(cam, lat, lon, alt, bearing);
                out.render = true;
            }
            Event::NorthUp => {
                self.north_up(cam);
                out.render = true;
            }
            Event::ToggleNorthUp => {
                if cam.north_up {
                    cam.north_up = false;
                } else {
                    self.north_up(cam);
                }
                out.render = true;
            }
            Event::Rotate(angle) => {
                cam.north_up = false;
                // Stack on a rotation already under way so repeated key
                // presses add up instead of restarting.
                let (lat, lon, bearing) = match &self.flight {
                    Some(f) => (f.lat, f.lon, f.bearing),
                    None => (cam.lat, cam.lon, cam.bearing),
                };
                self.flight = Some(Flight {
                    lat,
                    lon,
                    bearing: wrap_angle(bearing - angle),
                });
                out.render = true;
            }
            Event::Reset => {
                cam.north_up = true;
                self.fly_to(
                    cam,
                    HOME.0.to_radians(),
                    HOME.1.to_radians(),
                    HOME.2,
                    Some(0.0),
                );
                out.render = true;
            }
            Event::CancelInertia => self.velocity = (0.0, 0.0),
        }
        out
    }

    fn stop_motion(&mut self, cam: &Camera) {
        self.velocity = (0.0, 0.0);
        self.flight = None;
        self.anchor = None;
        self.target_alt = cam.alt;
    }

    fn zoom(&mut self, cam: &Camera, factor: f64, toward: Option<(f64, f64)>) {
        self.flight = None;
        self.target_alt = (self.target_alt * factor).clamp(MIN_ALT, MAX_ALT);
        self.anchor = toward.and_then(|(sx, sy)| {
            cam.pick(sx, sy)
                .map(|(lat, lon)| Anchor { lat, lon, sx, sy })
        });
    }

    /// Fly the view center to (lat, lon) and the altitude to `alt`. The
    /// bearing goes to `bearing`, or stays put in free mode and returns to
    /// north in north-up mode.
    fn fly_to(&mut self, cam: &Camera, lat: f64, lon: f64, alt: f64, bearing: Option<f64>) {
        self.stop_motion(cam);
        let bearing = match bearing {
            Some(b) => wrap_angle(b),
            None if cam.north_up => 0.0,
            None => cam.bearing,
        };
        self.flight = Some(Flight { lat, lon, bearing });
        self.target_alt = alt.clamp(MIN_ALT, MAX_ALT);
    }

    /// Lock north-up and turn the view to face north, keeping the center.
    fn north_up(&mut self, cam: &mut Camera) {
        cam.north_up = true;
        let target_alt = self.target_alt;
        self.fly_to(cam, cam.lat, cam.lon, cam.alt, Some(0.0));
        self.target_alt = target_alt;
    }

    fn pointer_down(&mut self, cam: &Camera, id: i32, x: f64, y: f64, t: f64) {
        self.stop_motion(cam);
        self.pointers.retain(|p| p.id != id);
        self.press = if self.pointers.is_empty() {
            Some(Press { x, y })
        } else {
            None // a second finger makes this a pinch
        };
        self.pointers.push(Pointer { id, x, y });
        self.pinch = false;
        self.twist_acc = 0.0;
        self.twisting = false;
        self.last_move_ms = t;
    }

    /// Returns whether the camera may have changed.
    fn pointer_move(
        &mut self,
        cam: &mut Camera,
        id: i32,
        (x, y): (f64, f64),
        t: f64,
        alt: bool,
    ) -> bool {
        if self
            .press
            .as_ref()
            .is_some_and(|p| (x - p.x).hypot(y - p.y) > TAP_SLOP_PX)
        {
            self.press = None;
        }
        let Some(idx) = self.pointers.iter().position(|p| p.id == id) else {
            return false;
        };
        let (px, py) = (self.pointers[idx].x, self.pointers[idx].y);

        if self.pointers.len() == 1 && alt {
            // Alt-drag: turn the view by the angle the pointer sweeps around
            // the screen center.
            let (cx, cy) = (cam.width * 0.5, cam.height * 0.5);
            let (ox, oy) = (px - cx, py - cy);
            let (nx, ny) = (x - cx, y - cy);
            let r = ox.hypot(oy).min(nx.hypot(ny));
            if r > 1.0 {
                let sweep = (ox * ny - oy * nx).atan2(ox * nx + oy * ny);
                cam.north_up = false;
                cam.twist(sweep * (r / ROTATE_DEADZONE_PX).min(1.0));
            }
            self.velocity = (0.0, 0.0);
        } else if self.pointers.len() == 1 {
            let (dx, dy) = (x - px, y - py);
            cam.pan_pixels(dx, dy);
            let dt = (t - self.last_move_ms).max(1.0);
            let (ix, iy) = (dx / dt, dy / dt);
            self.velocity = (
                0.75 * ix + 0.25 * self.velocity.0,
                0.75 * iy + 0.25 * self.velocity.1,
            );
        } else {
            // Pinch: zoom by finger distance, pan with the midpoint. Only the
            // first two pointers take part; further fingers are tracked so
            // they can take over when one of those lifts.
            let (a, b) = (&self.pointers[0], &self.pointers[1]);
            let old_mid = ((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
            let old_dist = (a.x - b.x).hypot(a.y - b.y);
            let old_angle = (b.y - a.y).atan2(b.x - a.x);
            self.pointers[idx].x = x;
            self.pointers[idx].y = y;
            let (a, b) = (&self.pointers[0], &self.pointers[1]);
            let new_mid = ((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
            let new_dist = (a.x - b.x).hypot(a.y - b.y);
            let new_angle = (b.y - a.y).atan2(b.x - a.x);
            let target = cam.pick(old_mid.0, old_mid.1);
            if old_dist > 1.0 && new_dist > 1.0 {
                cam.alt = (cam.alt * old_dist / new_dist).clamp(MIN_ALT, MAX_ALT);
                self.target_alt = cam.alt;
            }
            if old_dist > 1.0 && new_dist > 1.0 {
                // Fingers turning clockwise on screen turn the globe with them.
                let twist = wrap_angle(new_angle - old_angle);
                self.twist_acc += twist;
                if !self.twisting && self.twist_acc.abs() > TWIST_START {
                    self.twisting = true;
                    cam.north_up = false;
                    cam.twist(self.twist_acc);
                } else if self.twisting {
                    cam.twist(twist);
                }
            }
            match target {
                Some((lat, lon)) => cam.keep_point_at(lat, lon, new_mid.0, new_mid.1),
                None => cam.pan_pixels(new_mid.0 - old_mid.0, new_mid.1 - old_mid.1),
            }
            self.pinch = true;
        }
        self.pointers[idx].x = x;
        self.pointers[idx].y = y;
        self.last_move_ms = t;
        true
    }

    /// Returns the tap position when a lone press ends in place.
    fn pointer_up(&mut self, id: i32, t: f64, cancel: bool) -> Option<(f64, f64)> {
        self.pointers.retain(|p| p.id != id);
        let mut tap = None;
        if let Some(p) = self.press.take() {
            if self.pointers.is_empty() && !cancel {
                tap = Some((p.x, p.y));
            }
        }
        if self.pinch || t - self.last_move_ms > FLING_PAUSE_MS {
            self.velocity = (0.0, 0.0); // no fling after a pinch or a pause
        }
        if self.pointers.is_empty() {
            self.pinch = false;
        }
        tap
    }

    /// Advance animations; returns true if the camera moved.
    fn tick(&mut self, cam: &mut Camera, dt: f64) -> bool {
        let dt = dt.clamp(0.0, MAX_DT_MS);
        let mut moved = false;
        let ease = 1.0 - (-dt / 85.0).exp(); // ~85 ms time constant

        if let Some(f) = &self.flight {
            let dlat = f.lat - cam.lat;
            let dlon = wrap_angle(f.lon - cam.lon);
            let dbearing = wrap_angle(f.bearing - cam.bearing);
            if dlat.abs() < 1e-5 && dlon.abs() < 1e-5 && dbearing.abs() < 1e-5 {
                cam.lat = f.lat;
                cam.lon = wrap_angle(f.lon);
                cam.bearing = f.bearing;
                self.flight = None;
                moved = true;
            } else {
                // Slower than zoom so the flight reads as a single gesture.
                let k = 1.0 - (-dt / 160.0).exp();
                cam.lat += dlat * k;
                cam.lon = wrap_angle(cam.lon + dlon * k);
                cam.bearing = wrap_angle(cam.bearing + dbearing * k);
                moved = true;
            }
        }

        let (la, lt) = (cam.alt.ln(), self.target_alt.ln());
        if (lt - la).abs() > 1e-4 {
            cam.alt = (la + (lt - la) * ease).exp();
            if let Some(a) = &self.anchor {
                let (lat, lon, sx, sy) = (a.lat, a.lon, a.sx, a.sy);
                cam.keep_point_at(lat, lon, sx, sy);
            }
            moved = true;
        } else if cam.alt != self.target_alt {
            cam.alt = self.target_alt;
            self.anchor = None;
            moved = true;
        }

        if self.pointers.is_empty() {
            let (vx, vy) = self.velocity;
            if vx.hypot(vy) > 0.005 {
                cam.pan_pixels(vx * dt, vy * dt);
                let decay = (-dt / 325.0).exp();
                self.velocity = (vx * decay, vy * decay);
                moved = true;
            } else {
                self.velocity = (0.0, 0.0);
            }
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_motion_covers_drag_fling_flight_and_zoom_but_not_a_press() {
        let (mut c, mut k) = setup();
        assert!(!k.in_motion(&c));
        // A press that has not moved is not motion.
        down(&mut k, &mut c, 1, 500.0, 300.0, 0.0);
        assert!(!k.in_motion(&c));
        mv(&mut k, &mut c, 1, 540.0, 300.0, 10.0);
        assert!(k.in_motion(&c));
        mv(&mut k, &mut c, 1, 580.0, 300.0, 20.0);
        up(&mut k, &mut c, 1, 22.0);
        assert!(k.in_motion(&c), "fling");
        for _ in 0..400 {
            tick(&mut k, &mut c, 16.0);
        }
        assert!(!k.in_motion(&c), "came to rest");
        k.handle(
            &mut c,
            Event::FlyTo {
                lat: 10.0,
                lon: 50.0,
                alt: 0.5,
                bearing: None,
            },
        );
        assert!(k.in_motion(&c), "flight and zoom");
        for _ in 0..600 {
            tick(&mut k, &mut c, 16.0);
        }
        assert!(!k.in_motion(&c));
        k.handle(
            &mut c,
            Event::Zoom {
                factor: 0.5,
                toward: None,
            },
        );
        assert!(k.in_motion(&c), "zoom easing");
    }

    fn setup() -> (Camera, Controls) {
        let mut c = Camera::new(20.0, 10.0, 1.0);
        c.width = 1000.0;
        c.height = 700.0;
        let k = Controls::new(&c);
        (c, k)
    }

    fn down(k: &mut Controls, c: &mut Camera, id: i32, x: f64, y: f64, t: f64) -> Response {
        k.handle(c, Event::PointerDown { id, x, y, t })
    }
    fn mv(k: &mut Controls, c: &mut Camera, id: i32, x: f64, y: f64, t: f64) -> Response {
        k.handle(
            c,
            Event::PointerMove {
                id,
                x,
                y,
                t,
                alt: false,
            },
        )
    }
    fn up(k: &mut Controls, c: &mut Camera, id: i32, t: f64) -> Response {
        k.handle(
            c,
            Event::PointerUp {
                id,
                t,
                cancel: false,
            },
        )
    }
    fn tick(k: &mut Controls, c: &mut Camera, ms: f64) -> bool {
        k.handle(c, Event::Tick { dt: ms }).render
    }
    fn settle(k: &mut Controls, c: &mut Camera) {
        for _ in 0..600 {
            tick(k, c, 16.0);
        }
    }
    fn wheel(k: &mut Controls, c: &mut Camera, dy: f64, x: f64, y: f64) {
        k.handle(
            c,
            Event::Wheel {
                dy,
                x,
                y,
                mode: 0,
                ctrl: false,
            },
        );
    }
    fn sane(c: &Camera) -> bool {
        c.lat.is_finite()
            && c.lon.is_finite()
            && c.bearing.is_finite()
            && (-PI..=PI).contains(&c.bearing)
            && c.alt.is_finite()
            && c.alt >= MIN_ALT - 1e-9
            && c.alt <= MAX_ALT + 1e-9
            && c.lat.abs() <= 90f64.to_radians()
    }

    #[test]
    fn drag_keeps_surface_under_pointer() {
        let (mut c, mut k) = setup();
        let (x0, y0) = (500.0, 350.0);
        let grabbed = c.pick(x0, y0).unwrap();
        down(&mut k, &mut c, 1, x0, y0, 0.0);
        assert!(k.dragging());
        let r = mv(&mut k, &mut c, 1, x0 + 30.0, y0 + 10.0, 16.0);
        assert!(r.render);
        let (lat, lon) = c.pick(x0 + 30.0, y0 + 10.0).unwrap();
        // Exact at the screen center only; near it the error is small.
        assert!((lat - grabbed.0).abs() < 0.01);
        assert!((lon - grabbed.1).abs() < 0.01);
    }

    #[test]
    fn move_of_unknown_pointer_does_nothing() {
        let (mut c, mut k) = setup();
        let before = c;
        let r = mv(&mut k, &mut c, 9, 10.0, 10.0, 1.0);
        assert!(!r.render);
        assert_eq!((c.lat, c.lon), (before.lat, before.lon));
    }

    #[test]
    fn tap_reported_only_when_released_in_place() {
        let (mut c, mut k) = setup();
        down(&mut k, &mut c, 1, 100.0, 100.0, 0.0);
        assert_eq!(up(&mut k, &mut c, 1, 50.0).tap, Some((100.0, 100.0)));

        down(&mut k, &mut c, 1, 100.0, 100.0, 0.0);
        mv(&mut k, &mut c, 1, 100.0 + TAP_SLOP_PX + 1.0, 100.0, 10.0);
        assert_eq!(up(&mut k, &mut c, 1, 20.0).tap, None);

        down(&mut k, &mut c, 1, 100.0, 100.0, 0.0);
        let r = k.handle(
            &mut c,
            Event::PointerUp {
                id: 1,
                t: 10.0,
                cancel: true,
            },
        );
        assert_eq!(r.tap, None);

        // A second finger makes it a pinch, never a tap.
        down(&mut k, &mut c, 1, 100.0, 100.0, 0.0);
        down(&mut k, &mut c, 2, 200.0, 100.0, 1.0);
        assert_eq!(up(&mut k, &mut c, 2, 2.0).tap, None);
        assert_eq!(up(&mut k, &mut c, 1, 3.0).tap, None);
        assert!(!k.dragging());
    }

    #[test]
    fn fling_continues_and_decays() {
        let (mut c, mut k) = setup();
        down(&mut k, &mut c, 1, 500.0, 350.0, 0.0);
        for i in 1..=5 {
            mv(
                &mut k,
                &mut c,
                1,
                500.0 + 20.0 * i as f64,
                350.0,
                16.0 * i as f64,
            );
        }
        up(&mut k, &mut c, 1, 81.0);
        assert!(k.animating());
        let lon0 = c.lon;
        assert!(tick(&mut k, &mut c, 16.0));
        assert!(c.lon < lon0, "dragging right moves the view west");
        settle(&mut k, &mut c);
        assert!(!k.animating());
        assert!(!tick(&mut k, &mut c, 16.0), "idle ticks do no work");
    }

    #[test]
    fn no_fling_after_a_pause() {
        let (mut c, mut k) = setup();
        down(&mut k, &mut c, 1, 500.0, 350.0, 0.0);
        mv(&mut k, &mut c, 1, 540.0, 350.0, 16.0);
        up(&mut k, &mut c, 1, 16.0 + FLING_PAUSE_MS + 1.0);
        assert!(!k.animating());
    }

    #[test]
    fn pinch_zooms_by_finger_distance() {
        let (mut c, mut k) = setup();
        down(&mut k, &mut c, 1, 400.0, 350.0, 0.0);
        down(&mut k, &mut c, 2, 600.0, 350.0, 0.0);
        let alt = c.alt;
        mv(&mut k, &mut c, 2, 800.0, 350.0, 16.0); // distance 200 -> 400
        assert!((c.alt - alt / 2.0).abs() < 1e-9);
        // The release after a pinch kills any velocity.
        up(&mut k, &mut c, 2, 17.0);
        up(&mut k, &mut c, 1, 18.0);
        assert!(!k.animating());
    }

    #[test]
    fn pinch_alt_stays_in_range() {
        let (mut c, mut k) = setup();
        down(&mut k, &mut c, 1, 499.0, 350.0, 0.0);
        down(&mut k, &mut c, 2, 501.0, 350.0, 0.0);
        mv(&mut k, &mut c, 2, 5000.0, 350.0, 16.0);
        assert!(sane(&c));
        mv(&mut k, &mut c, 2, 502.0, 350.0, 32.0);
        mv(&mut k, &mut c, 2, 502.0, 350.0, 48.0);
        assert!(sane(&c));
    }

    #[test]
    fn pinch_with_three_pointers_is_stable() {
        let (mut c, mut k) = setup();
        down(&mut k, &mut c, 1, 400.0, 350.0, 0.0);
        down(&mut k, &mut c, 2, 600.0, 350.0, 0.0);
        down(&mut k, &mut c, 3, 500.0, 200.0, 0.0);
        let before = c;
        // The third finger alone does not move the camera.
        mv(&mut k, &mut c, 3, 520.0, 120.0, 16.0);
        assert_eq!((c.lat, c.lon, c.alt), (before.lat, before.lon, before.alt));
        // The first two still pinch.
        mv(&mut k, &mut c, 2, 700.0, 350.0, 32.0);
        assert!(c.alt < before.alt);
        assert!(sane(&c));
        // Lift the first finger: the third takes its slot with no jump.
        up(&mut k, &mut c, 1, 40.0);
        let after = c;
        mv(&mut k, &mut c, 3, 520.0, 120.0, 48.0); // no actual movement
        assert!((c.lat - after.lat).abs() < 1e-9);
        assert!((c.lon - after.lon).abs() < 1e-9);
        assert!((c.alt - after.alt).abs() < 1e-9);
        up(&mut k, &mut c, 2, 50.0);
        up(&mut k, &mut c, 3, 50.0);
        assert!(!k.dragging());
        assert!(!k.animating());
    }

    #[test]
    fn lifting_one_finger_mid_pinch_then_dragging_does_not_jump() {
        let (mut c, mut k) = setup();
        down(&mut k, &mut c, 1, 400.0, 350.0, 0.0);
        down(&mut k, &mut c, 2, 600.0, 350.0, 0.0);
        mv(&mut k, &mut c, 1, 380.0, 340.0, 16.0);
        mv(&mut k, &mut c, 2, 640.0, 360.0, 32.0);
        up(&mut k, &mut c, 1, 40.0);
        let settled = c;
        // The remaining finger sits where the last move left it; the first
        // drag move is relative to that, so a tiny step is a tiny pan.
        mv(&mut k, &mut c, 2, 641.0, 360.0, 48.0);
        let step = ((c.lat - settled.lat).powi(2) + (c.lon - settled.lon).powi(2)).sqrt();
        assert!(step < 0.01, "jumped by {step} rad");
        assert_eq!(c.alt, settled.alt, "drag after pinch must not zoom");
        // And no fling: the pinch flag survives until all fingers are up.
        up(&mut k, &mut c, 2, 49.0);
        assert!(!k.animating());
        // A fresh drag may fling again.
        down(&mut k, &mut c, 1, 500.0, 350.0, 100.0);
        mv(&mut k, &mut c, 1, 540.0, 350.0, 116.0);
        up(&mut k, &mut c, 1, 117.0);
        assert!(k.animating());
    }

    #[test]
    fn duplicate_pointer_down_replaces_the_pointer() {
        let (mut c, mut k) = setup();
        down(&mut k, &mut c, 1, 100.0, 100.0, 0.0);
        down(&mut k, &mut c, 1, 300.0, 300.0, 1.0);
        let before = c;
        mv(&mut k, &mut c, 1, 300.0, 300.0, 2.0);
        assert!((c.lat - before.lat).abs() < 1e-12 && (c.lon - before.lon).abs() < 1e-12);
        up(&mut k, &mut c, 1, 3.0);
        assert!(!k.dragging());
    }

    #[test]
    fn wheel_zoom_keeps_the_anchor_under_the_cursor() {
        let (mut c, mut k) = setup();
        let (sx, sy) = (620.0, 260.0);
        let (lat, lon) = c.pick(sx, sy).unwrap();
        wheel(&mut k, &mut c, -400.0, sx, sy);
        let target = k.target_alt;
        assert!(target < c.alt);
        let mut prev = c.alt;
        for _ in 0..200 {
            tick(&mut k, &mut c, 16.0);
            assert!(c.alt <= prev + 1e-12, "zoom is monotonic");
            prev = c.alt;
            let (px, py) = c.project(lat, lon).unwrap();
            assert!((px - sx).abs() < 0.5 && (py - sy).abs() < 0.5);
        }
        assert!((c.alt - target).abs() < 1e-6);
        assert!(!tick(&mut k, &mut c, 16.0), "idle ticks do no work");
    }

    #[test]
    fn wheel_modes_scale_deltas() {
        let ev = |dy, mode, ctrl| Event::Wheel {
            dy,
            x: 500.0,
            y: 350.0,
            mode,
            ctrl,
        };
        let target = |e| {
            let (mut c, mut k) = setup();
            k.handle(&mut c, e);
            k.target_alt
        };
        assert!((target(ev(1.0, 1, false)) - target(ev(16.0, 0, false))).abs() < 1e-12);
        // ctrl (trackpad pinch) is more sensitive.
        assert!(target(ev(16.0, 0, true)) > target(ev(16.0, 0, false)));
        // Huge deltas are clamped to one e-fold.
        assert!((target(ev(1e9, 2, true)) - 1.0f64.exp()).abs() < 1e-9);
    }

    #[test]
    fn wheel_off_the_globe_zooms_without_an_anchor() {
        let (mut c, mut k) = setup();
        c.alt = 3.0;
        k.target_alt = 3.0;
        assert!(c.pick(2.0, 2.0).is_none());
        wheel(&mut k, &mut c, -400.0, 2.0, 2.0);
        assert!(k.anchor.is_none());
        let (lat, lon) = (c.lat, c.lon);
        settle(&mut k, &mut c);
        assert!(c.alt < 3.0);
        assert_eq!((c.lat, c.lon), (lat, lon), "no anchor means no pan");
    }

    #[test]
    fn anchor_leaving_the_globe_mid_animation_is_safe() {
        // Zoom out from a point near the limb: as the camera rises the
        // anchored point slips toward (or over) the horizon.
        let (mut c, mut k) = setup();
        c.alt = 0.1;
        k.target_alt = 0.1;
        let mut x = 500.0;
        while c.pick(x + 1.0, 350.0).is_some() {
            x += 1.0;
        }
        assert!(x > 500.0, "found a pickable point near the limb");
        k.handle(
            &mut c,
            Event::Zoom {
                factor: 6.0,
                toward: Some((x, 350.0)),
            },
        );
        assert!(k.anchor.is_some());
        for _ in 0..400 {
            tick(&mut k, &mut c, 16.0);
            assert!(sane(&c));
        }
        assert!(c.alt > 0.5);
        assert!(k.anchor.is_none(), "anchor is dropped once the zoom lands");
    }

    #[test]
    fn zoom_clamping() {
        let (mut c, mut k) = setup();
        for _ in 0..40 {
            k.handle(
                &mut c,
                Event::Zoom {
                    factor: 0.5,
                    toward: None,
                },
            );
        }
        settle(&mut k, &mut c);
        assert!((c.alt - MIN_ALT).abs() < 1e-6);
        for _ in 0..40 {
            k.handle(
                &mut c,
                Event::Zoom {
                    factor: 2.0,
                    toward: None,
                },
            );
        }
        settle(&mut k, &mut c);
        assert!((c.alt - MAX_ALT).abs() < 1e-6);
    }

    #[test]
    fn fling_is_stopped_by_pointer_down() {
        let (mut c, mut k) = setup();
        k.handle(&mut c, Event::Nudge(Direction::Right));
        assert!(k.animating());
        down(&mut k, &mut c, 1, 10.0, 10.0, 0.0);
        assert!(!k.animating());
        let before = c;
        tick(&mut k, &mut c, 16.0);
        assert_eq!((c.lat, c.lon), (before.lat, before.lon));
    }

    #[test]
    fn fling_and_wheel_zoom_run_together() {
        let (mut c, mut k) = setup();
        k.handle(&mut c, Event::Nudge(Direction::Left));
        wheel(&mut k, &mut c, -400.0, 500.0, 350.0);
        let (lon0, alt0) = (c.lon, c.alt);
        assert!(tick(&mut k, &mut c, 16.0));
        assert!(c.lon != lon0, "fling keeps panning");
        assert!(c.alt < alt0, "zoom keeps easing");
        settle(&mut k, &mut c);
        assert!(sane(&c));
        assert!(!k.animating());
    }

    #[test]
    fn fly_to_cancels_fling_and_lands_on_target() {
        let (mut c, mut k) = setup();
        k.handle(&mut c, Event::Nudge(Direction::Left));
        let r = k.handle(
            &mut c,
            Event::FlyTo {
                lat: 0.5,
                lon: -2.0,
                alt: 0.3,
                bearing: None,
            },
        );
        assert!(r.render);
        assert_eq!(k.velocity, (0.0, 0.0));
        settle(&mut k, &mut c);
        assert!((c.lat - 0.5).abs() < 1e-4);
        assert!(wrap_angle(c.lon + 2.0).abs() < 1e-4);
        assert!((c.alt - 0.3).abs() < 1e-6);
        assert!(!k.animating());
    }

    #[test]
    fn fly_to_takes_the_short_way_across_the_seam() {
        let (mut c, mut k) = setup();
        c.lon = 179f64.to_radians();
        let lat0 = c.lat;
        k.handle(
            &mut c,
            Event::FlyTo {
                lat: lat0,
                lon: (-179f64).to_radians(),
                alt: 1.0,
                bearing: None,
            },
        );
        tick(&mut k, &mut c, 16.0);
        let step = wrap_angle(c.lon - 179f64.to_radians());
        assert!(step > 0.0 && step < 0.05, "must go east, not the long way");
    }

    #[test]
    fn fly_to_clamps_altitude() {
        let (mut c, mut k) = setup();
        let fly = |alt| Event::FlyTo {
            lat: 0.0,
            lon: 0.0,
            alt,
            bearing: None,
        };
        k.handle(&mut c, fly(1e9));
        settle(&mut k, &mut c);
        assert!((c.alt - MAX_ALT).abs() < 1e-6);
        k.handle(&mut c, fly(-5.0));
        settle(&mut k, &mut c);
        assert!((c.alt - MIN_ALT).abs() < 1e-6);
    }

    #[test]
    fn wheel_cancels_a_flight_and_fly_to_cancels_a_zoom_anchor() {
        let (mut c, mut k) = setup();
        k.handle(
            &mut c,
            Event::FlyTo {
                lat: 1.0,
                lon: 1.0,
                alt: 0.5,
                bearing: None,
            },
        );
        assert!(k.flight.is_some());
        wheel(&mut k, &mut c, 100.0, 500.0, 350.0);
        assert!(k.flight.is_none());
        assert!(k.anchor.is_some());
        k.handle(
            &mut c,
            Event::FlyTo {
                lat: 0.0,
                lon: 0.0,
                alt: 1.0,
                bearing: None,
            },
        );
        assert!(k.anchor.is_none());
        assert!(k.flight.is_some());
        settle(&mut k, &mut c);
        assert!(sane(&c));
    }

    #[test]
    fn double_click_flies_closer_or_farther() {
        let (mut c, mut k) = setup();
        let (lat, lon) = c.pick(600.0, 300.0).unwrap();
        let dbl = |x, y, shift| Event::DoubleClick { x, y, shift };
        k.handle(&mut c, dbl(600.0, 300.0, false));
        settle(&mut k, &mut c);
        assert!((c.alt - 0.4).abs() < 1e-6);
        assert!((c.lat - lat).abs() < 1e-4 && wrap_angle(c.lon - lon).abs() < 1e-4);
        k.handle(&mut c, dbl(500.0, 350.0, true));
        settle(&mut k, &mut c);
        assert!((c.alt - 1.0).abs() < 1e-6);
        // A miss does nothing.
        let (mut c, mut k) = setup();
        let r = k.handle(&mut c, dbl(0.0, 0.0, false));
        assert!(!r.render && !k.animating());
    }

    #[test]
    fn reset_flies_home() {
        let (mut c, mut k) = setup();
        k.handle(
            &mut c,
            Event::FlyTo {
                lat: -1.0,
                lon: 2.0,
                alt: 0.1,
                bearing: None,
            },
        );
        settle(&mut k, &mut c);
        assert!(k.handle(&mut c, Event::Reset).render);
        settle(&mut k, &mut c);
        assert!((c.lat - HOME.0.to_radians()).abs() < 1e-4);
        assert!(wrap_angle(c.lon - HOME.1.to_radians()).abs() < 1e-4);
        assert!((c.alt - HOME.2.clamp(MIN_ALT, MAX_ALT)).abs() < 1e-6);
    }

    #[test]
    fn cancel_inertia_only_drops_velocity() {
        let (mut c, mut k) = setup();
        k.handle(&mut c, Event::Nudge(Direction::Up));
        k.handle(
            &mut c,
            Event::Zoom {
                factor: 0.5,
                toward: None,
            },
        );
        k.handle(&mut c, Event::CancelInertia);
        assert_eq!(k.velocity, (0.0, 0.0));
        assert!(tick(&mut k, &mut c, 16.0), "zoom still animates");
    }

    #[test]
    fn non_finite_events_are_ignored() {
        let (mut c, mut k) = setup();
        let evs = [
            Event::PointerDown {
                id: 1,
                x: f64::NAN,
                y: 0.0,
                t: 0.0,
            },
            Event::Wheel {
                dy: f64::INFINITY,
                x: 1.0,
                y: 1.0,
                mode: 0,
                ctrl: false,
            },
            Event::Tick { dt: f64::NAN },
            Event::Zoom {
                factor: 0.0,
                toward: None,
            },
            Event::Zoom {
                factor: f64::NAN,
                toward: None,
            },
            Event::FlyTo {
                lat: f64::NAN,
                lon: 0.0,
                alt: 1.0,
                bearing: None,
            },
        ];
        let before = c;
        for e in evs {
            assert_eq!(k.handle(&mut c, e), Response::default());
        }
        assert!(!k.dragging() && !k.animating());
        assert_eq!((c.lat, c.lon, c.alt), (before.lat, before.lon, before.alt));
    }

    fn mv_alt(k: &mut Controls, c: &mut Camera, id: i32, x: f64, y: f64, t: f64) {
        k.handle(
            c,
            Event::PointerMove {
                id,
                x,
                y,
                t,
                alt: true,
            },
        );
    }

    #[test]
    fn two_finger_twist_rotates_the_view_and_unlocks_north_up() {
        let (mut c, mut k) = setup();
        assert!(c.north_up);
        let center = (c.lat, c.lon);
        // Two fingers either side of the screen center, then turn clockwise.
        down(&mut k, &mut c, 1, 400.0, 350.0, 0.0);
        down(&mut k, &mut c, 2, 600.0, 350.0, 0.0);
        let mut t = 0.0;
        for i in 1..=20 {
            t += 16.0;
            let a = i as f64 * 0.02; // 0.4 rad in total
            let (dx, dy) = (100.0 * a.cos(), 100.0 * a.sin());
            mv(&mut k, &mut c, 1, 500.0 - dx, 350.0 - dy, t);
            mv(&mut k, &mut c, 2, 500.0 + dx, 350.0 + dy, t);
        }
        assert!(!c.north_up, "twisting unlocks north-up");
        assert!(
            (c.bearing + 0.4).abs() < 0.03,
            "clockwise twist lowers the bearing: {}",
            c.bearing
        );
        // The pinch midpoint stayed put, so the center barely moved.
        assert!((c.lat - center.0).abs() < 0.01 && (c.lon - center.1).abs() < 0.01);
        assert!(sane(&c));
    }

    #[test]
    fn a_plain_pinch_does_not_unlock_north_up() {
        let (mut c, mut k) = setup();
        down(&mut k, &mut c, 1, 450.0, 350.0, 0.0);
        down(&mut k, &mut c, 2, 550.0, 350.0, 0.0);
        // Spread apart with a sloppy few degrees of twist.
        mv(&mut k, &mut c, 2, 600.0, 358.0, 16.0);
        mv(&mut k, &mut c, 1, 400.0, 346.0, 32.0);
        assert!(c.north_up);
        assert_eq!(c.bearing, 0.0);
    }

    #[test]
    fn twist_keeps_the_pinch_midpoint_under_the_fingers() {
        let (mut c, mut k) = setup();
        let mid = (420.0, 300.0);
        let grabbed = c.pick(mid.0, mid.1).unwrap();
        down(&mut k, &mut c, 1, mid.0 - 60.0, mid.1, 0.0);
        down(&mut k, &mut c, 2, mid.0 + 60.0, mid.1, 0.0);
        for i in 1..=15 {
            let a = i as f64 * 0.03;
            let (dx, dy) = (60.0 * a.cos(), 60.0 * a.sin());
            mv(&mut k, &mut c, 1, mid.0 - dx, mid.1 - dy, i as f64 * 16.0);
            mv(&mut k, &mut c, 2, mid.0 + dx, mid.1 + dy, i as f64 * 16.0);
        }
        assert!(!c.north_up);
        let (px, py) = c.project(grabbed.0, grabbed.1).unwrap();
        assert!(
            (px - mid.0).abs() < 3.0 && (py - mid.1).abs() < 3.0,
            "({px},{py})"
        );
    }

    #[test]
    fn alt_drag_turns_the_view_about_the_center() {
        let (mut c, mut k) = setup();
        let center = (c.lat, c.lon);
        // Press right of the center and sweep downward: clockwise on screen.
        down(&mut k, &mut c, 1, 800.0, 350.0, 0.0);
        let mut t = 0.0;
        for i in 1..=10 {
            t += 16.0;
            let a = i as f64 * 0.05;
            mv_alt(
                &mut k,
                &mut c,
                1,
                500.0 + 300.0 * a.cos(),
                350.0 + 300.0 * a.sin(),
                t,
            );
        }
        assert!(!c.north_up);
        assert!((c.bearing + 0.5).abs() < 0.02, "{}", c.bearing);
        assert_eq!((c.lat, c.lon), center, "alt-drag never pans");
        up(&mut k, &mut c, 1, t + 1.0);
        assert_eq!(k.velocity, (0.0, 0.0), "no fling from a rotation");
    }

    #[test]
    fn rotation_inputs_count_as_motion() {
        let (mut c, mut k) = setup();
        down(&mut k, &mut c, 1, 800.0, 350.0, 0.0);
        mv_alt(&mut k, &mut c, 1, 790.0, 400.0, 16.0);
        mv_alt(&mut k, &mut c, 1, 770.0, 450.0, 32.0);
        assert!(k.in_motion(&c), "alt-drag");
        up(&mut k, &mut c, 1, 33.0);
        settle(&mut k, &mut c);
        assert!(!k.in_motion(&c));
        down(&mut k, &mut c, 1, 400.0, 350.0, 0.0);
        down(&mut k, &mut c, 2, 600.0, 350.0, 0.0);
        mv(&mut k, &mut c, 2, 590.0, 420.0, 16.0);
        assert!(k.in_motion(&c), "twist");
        up(&mut k, &mut c, 2, 20.0);
        up(&mut k, &mut c, 1, 21.0);
        settle(&mut k, &mut c);
        k.handle(&mut c, Event::Rotate(ROTATE_STEP));
        assert!(k.in_motion(&c), "keyboard rotation");
        settle(&mut k, &mut c);
        k.handle(&mut c, Event::NorthUp);
        assert!(k.in_motion(&c), "compass");
    }

    #[test]
    fn alt_drag_near_the_center_is_damped_and_finite() {
        let (mut c, mut k) = setup();
        down(&mut k, &mut c, 1, 502.0, 350.0, 0.0);
        mv_alt(&mut k, &mut c, 1, 498.0, 351.0, 16.0);
        mv_alt(&mut k, &mut c, 1, 500.0, 350.0, 32.0); // exactly the center
        assert!(sane(&c));
        assert!(c.bearing.abs() < 1.0);
    }

    #[test]
    fn compass_animates_back_to_north_up_and_locks() {
        let (mut c, mut k) = setup();
        c.north_up = false;
        c.bearing = 2.0;
        let (lat, lon, alt) = (c.lat, c.lon, c.alt);
        let r = k.handle(&mut c, Event::NorthUp);
        assert!(r.render && c.north_up);
        assert_eq!(c.bearing, 2.0, "animates, does not jump");
        tick(&mut k, &mut c, 16.0);
        assert!(c.bearing.abs() < 2.0 && c.bearing > 0.0, "{}", c.bearing);
        settle(&mut k, &mut c);
        assert_eq!(c.bearing, 0.0);
        assert!((c.lat - lat).abs() < 1e-9 && (c.lon - lon).abs() < 1e-9);
        assert!((c.alt - alt).abs() < 1e-9, "the compass never zooms");
        assert!(!k.animating());
    }

    #[test]
    fn compass_takes_the_short_way_round() {
        let (mut c, mut k) = setup();
        c.north_up = false;
        c.bearing = -3.0; // just short of the seam at 180 degrees
        k.handle(&mut c, Event::NorthUp);
        tick(&mut k, &mut c, 16.0);
        assert!(c.bearing > -3.0 && c.bearing < 0.0, "{}", c.bearing);
        let (mut c, mut k) = setup();
        c.north_up = false;
        c.bearing = 3.0;
        k.handle(&mut c, Event::NorthUp);
        tick(&mut k, &mut c, 16.0);
        assert!(c.bearing < 3.0 && c.bearing > 0.0);
    }

    #[test]
    fn compass_restores_north_up_after_tilting_over_a_pole() {
        let (mut c, mut k) = setup();
        c.lat = 89.0f64.to_radians();
        down(&mut k, &mut c, 1, 500.0, 100.0, 0.0);
        let mut y = 100.0;
        while c.bearing.abs() < 3.0 && y < 600.0 {
            y += 1.0;
            mv(&mut k, &mut c, 1, 500.0, y, y);
        }
        assert!(c.bearing.abs() > 3.0, "tilted over the pole");
        up(&mut k, &mut c, 1, 1e4);
        k.handle(&mut c, Event::CancelInertia);
        k.handle(&mut c, Event::NorthUp);
        settle(&mut k, &mut c);
        assert_eq!(c.bearing, 0.0);
        assert!(c.north_up && sane(&c));
    }

    #[test]
    fn toggle_north_up_lock() {
        let (mut c, mut k) = setup();
        k.handle(&mut c, Event::ToggleNorthUp);
        assert!(!c.north_up, "first press unlocks");
        // Free mode: a drag does not snap the roll.
        c.bearing = 0.7;
        down(&mut k, &mut c, 1, 500.0, 350.0, 0.0);
        mv(&mut k, &mut c, 1, 540.0, 380.0, 16.0);
        up(&mut k, &mut c, 1, 17.0);
        k.handle(&mut c, Event::CancelInertia);
        assert!((c.bearing - 0.7).abs() < 0.05, "{}", c.bearing);
        k.handle(&mut c, Event::ToggleNorthUp);
        assert!(c.north_up, "second press locks and turns to north");
        settle(&mut k, &mut c);
        assert_eq!(c.bearing, 0.0);
    }

    #[test]
    fn rotate_steps_stack_and_unlock() {
        let (mut c, mut k) = setup();
        k.handle(&mut c, Event::Rotate(ROTATE_STEP));
        k.handle(&mut c, Event::Rotate(ROTATE_STEP));
        assert!(!c.north_up);
        settle(&mut k, &mut c);
        assert!(
            (c.bearing + 2.0 * ROTATE_STEP).abs() < 1e-4,
            "{}",
            c.bearing
        );
        k.handle(&mut c, Event::Rotate(-4.0 * ROTATE_STEP));
        settle(&mut k, &mut c);
        assert!((c.bearing - 2.0 * ROTATE_STEP).abs() < 1e-4);
    }

    #[test]
    fn fly_to_keeps_or_sets_the_bearing() {
        let (mut c, mut k) = setup();
        c.north_up = false;
        c.bearing = 1.0;
        k.handle(
            &mut c,
            Event::FlyTo {
                lat: 0.2,
                lon: 0.3,
                alt: 1.0,
                bearing: None,
            },
        );
        settle(&mut k, &mut c);
        assert!(
            (c.bearing - 1.0).abs() < 1e-9,
            "free mode keeps the bearing"
        );
        k.handle(
            &mut c,
            Event::FlyTo {
                lat: 0.2,
                lon: 0.3,
                alt: 1.0,
                bearing: Some(-0.5),
            },
        );
        settle(&mut k, &mut c);
        assert!((c.bearing + 0.5).abs() < 1e-9 && !c.north_up);
        k.handle(
            &mut c,
            Event::FlyTo {
                lat: 0.2,
                lon: 0.3,
                alt: 1.0,
                bearing: Some(0.0),
            },
        );
        assert!(c.north_up, "bearing 0 means north-up");
        settle(&mut k, &mut c);
        // A north-up view flying somewhere returns to north.
        c.north_up = true;
        c.bearing = PI;
        k.handle(
            &mut c,
            Event::FlyTo {
                lat: 0.1,
                lon: 0.1,
                alt: 1.0,
                bearing: None,
            },
        );
        settle(&mut k, &mut c);
        assert_eq!(c.bearing, 0.0);
    }

    #[test]
    fn reset_restores_north_up() {
        let (mut c, mut k) = setup();
        c.north_up = false;
        c.bearing = 1.3;
        k.handle(&mut c, Event::Reset);
        assert!(c.north_up);
        settle(&mut k, &mut c);
        assert_eq!(c.bearing, 0.0);
    }

    #[test]
    fn fly_to_a_pole_lands_exactly_on_it() {
        for (lat, sign) in [(90.0, 1.0), (-90.0, -1.0)] {
            let (mut c, mut k) = setup();
            k.handle(
                &mut c,
                Event::FlyTo {
                    lat: f64::to_radians(lat),
                    lon: 0.4,
                    alt: 0.5,
                    bearing: None,
                },
            );
            settle(&mut k, &mut c);
            assert!((c.lat - sign * PI / 2.0).abs() < 1e-12);
            let (plat, _) = c.pick(c.width * 0.5, c.height * 0.5).unwrap();
            assert!((plat - sign * PI / 2.0).abs() < 1e-9, "centered exactly");
        }
    }

    #[test]
    fn dragging_through_the_pole_with_inertia_stays_sane() {
        let (mut c, mut k) = setup();
        c.lat = 88.0f64.to_radians();
        down(&mut k, &mut c, 1, 500.0, 100.0, 0.0);
        for i in 1..=30 {
            mv(
                &mut k,
                &mut c,
                1,
                500.0,
                100.0 + i as f64 * 12.0,
                i as f64 * 8.0,
            );
            assert!(sane(&c));
        }
        up(&mut k, &mut c, 1, 241.0);
        for _ in 0..200 {
            tick(&mut k, &mut c, 16.0);
            assert!(sane(&c));
        }
    }

    /// Small deterministic xorshift generator; no dependency needed.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 / (1u64 << 53) as f64
        }
        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.unit()
        }
    }

    #[test]
    fn random_event_sequences_never_produce_nan_or_infinity() {
        for seed in 1..=200u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let (mut c, mut k) = setup();
            if seed % 3 == 0 {
                // Start on a pole, in either mode.
                c.lat = if seed % 2 == 0 { PI / 2.0 } else { -PI / 2.0 };
                c.north_up = seed % 4 != 0;
            }
            let mut t = 0.0;
            for _ in 0..400 {
                t += rng.range(0.0, 40.0);
                // Coordinates may be far outside the viewport.
                let (x, y) = (rng.range(-500.0, 1500.0), rng.range(-500.0, 1200.0));
                let id = (rng.next() % 4) as i32;
                let ev = match rng.next() % 19 {
                    0 => Event::PointerDown { id, x, y, t },
                    1..=3 => Event::PointerMove {
                        id,
                        x,
                        y,
                        t,
                        alt: rng.unit() < 0.3,
                    },
                    4 => Event::PointerUp {
                        id,
                        t,
                        cancel: rng.unit() < 0.5,
                    },
                    5 => Event::Wheel {
                        dy: rng.range(-3000.0, 3000.0),
                        x,
                        y,
                        mode: (rng.next() % 3) as u32,
                        ctrl: rng.unit() < 0.5,
                    },
                    6..=8 => Event::Tick {
                        dt: rng.range(0.0, 150.0),
                    },
                    9 => Event::Zoom {
                        factor: rng.range(0.1, 5.0),
                        toward: (rng.unit() < 0.5).then_some((x, y)),
                    },
                    10 => Event::DoubleClick {
                        x,
                        y,
                        shift: rng.unit() < 0.5,
                    },
                    11 => Event::Nudge(match rng.next() % 4 {
                        0 => Direction::Left,
                        1 => Direction::Right,
                        2 => Direction::Up,
                        _ => Direction::Down,
                    }),
                    12 => Event::FlyTo {
                        lat: rng.range(-1.5, 1.5),
                        lon: rng.range(-7.0, 7.0),
                        alt: rng.range(-1.0, 10.0),
                        bearing: None,
                    },
                    14 => Event::Rotate(rng.range(-7.0, 7.0)),
                    15 => Event::NorthUp,
                    16 => Event::ToggleNorthUp,
                    17 => Event::FlyTo {
                        lat: if rng.unit() < 0.5 {
                            PI / 2.0
                        } else {
                            -PI / 2.0
                        },
                        lon: rng.range(-7.0, 7.0),
                        alt: rng.range(0.0, 3.0),
                        bearing: (rng.unit() < 0.5).then(|| rng.range(-9.0, 9.0)),
                    },
                    _ => {
                        if rng.unit() < 0.33 {
                            Event::Reset
                        } else {
                            Event::CancelInertia
                        }
                    }
                };
                k.handle(&mut c, ev);
                assert!(sane(&c), "seed {seed}: bad camera {c:?} after {ev:?}");
                assert!(
                    k.velocity.0.is_finite() && k.velocity.1.is_finite(),
                    "seed {seed}: bad velocity"
                );
            }
            // Lift everything and let it settle: must come to rest.
            for id in 0..4 {
                up(&mut k, &mut c, id, t + 1.0);
            }
            settle(&mut k, &mut c);
            assert!(sane(&c));
            assert!(!k.dragging());
            assert!(!k.animating(), "seed {seed}: never came to rest");
        }
    }
}
