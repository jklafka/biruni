//! The solar system view: the Sun, planets and the Moon in space, seen by a
//! camera that orbits one of them. Browser-free (tested natively).
//!
//! Coordinate conventions (see AGENTS.md):
//!
//! - **Solar space** is heliocentric, in astronomical units, on the J2000
//!   ecliptic. It mirrors the sky view's equatorial space: +Y is the north
//!   ecliptic pole, ecliptic longitude 0 (the March equinox) lies on +Z and
//!   longitude 90 degrees on +X. `ephemeris` works in `(x to the equinox,
//!   y, z to the pole)`; [`ecl_to_solar`] moves those axes to `(y, z, x)`,
//!   a cyclic swap, so the frame stays right-handed. Solar space and sky
//!   equatorial space differ by a turn of the obliquity about +Z
//!   ([`SOLAR_TO_EQ`]), so the star catalog and the Milky Way draw behind
//!   the planets with the sky view's own code.
//! - **The camera** looks at a focus (normally the center of its target
//!   body) from `dist` au away. Its look direction is an azimuth and an
//!   altitude in solar space, exactly as the sky camera's is in horizontal
//!   space (azimuth 0 on -Z, 90 degrees on +X; altitude from the ecliptic
//!   plane), and there is no roll: ecliptic north is always up the screen.
//!   The camera stands at `focus - dist * look`.
//! - **The projection** is the sky view's stereographic one
//!   ([`SkyCamera::project`] / [`SkyCamera::pick`]), applied to directions
//!   from the camera. It is conformal, so a sphere's outline is an exact
//!   circle on screen ([`SolarCamera::disk`]).
//! - **Camera-relative coordinates.** Positions span 30 au and more, and an
//!   `f32` resolves 30 au only to about 270 km, more than the size of a
//!   small moon, while the camera may stand a few thousand km from a planet.
//!   So every position stays `f64` on the CPU, the camera's position is
//!   subtracted in `f64` ([`SolarCamera::relative`]), and only the small
//!   camera-relative result, turned into camera space, is cast to `f32` for
//!   the GPU. Nothing in a shader ever sees a heliocentric coordinate.
//!
//! Time is the time scrubber's (UTC Unix seconds), turned into TT centuries
//! by `ephemeris::centuries_tt_from_unix`.

use crate::ephemeris::{self, Planet};
use crate::moon;
use crate::sky::{horiz_vec, mul, transpose, Look, Mat3, SkyCamera};
use std::f64::consts::PI;

/// Kilometers in an astronomical unit.
pub const AU_KM: f64 = 149_597_870.7;
/// The Earth's mass over the Moon's, to place the Earth and the Moon about
/// their barycenter (which is what `ephemeris` gives for "Earth").
const EARTH_MOON_MASS_RATIO: f64 = 81.300_56;

/// Field of view across the shorter screen axis.
pub const FOV: f64 = 50.0 * PI / 180.0;
/// The farthest the camera goes from its focus, au: past Neptune's orbit
/// with room to see it whole.
pub const MAX_DIST: f64 = 250.0;
/// The view a reset returns to: the Sun from 3.5 au, from 30 degrees
/// above the ecliptic, so the inner planets fill the screen.
pub const HOME: (Body, f64, f64, f64) = (Body::Sun, PI, 30.0 * PI / 180.0, 3.5);
/// How long a fly-to takes, ms.
pub const FLIGHT_MS: f64 = 2_500.0;

/// A body the camera can orbit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Body {
    Sun,
    Mercury,
    Venus,
    Earth,
    Moon,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
}

impl Body {
    pub const ALL: [Body; 10] = [
        Body::Sun,
        Body::Mercury,
        Body::Venus,
        Body::Earth,
        Body::Moon,
        Body::Mars,
        Body::Jupiter,
        Body::Saturn,
        Body::Uranus,
        Body::Neptune,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Body::Sun => "Sun",
            Body::Mercury => "Mercury",
            Body::Venus => "Venus",
            Body::Earth => "Earth",
            Body::Moon => "Moon",
            Body::Mars => "Mars",
            Body::Jupiter => "Jupiter",
            Body::Saturn => "Saturn",
            Body::Uranus => "Uranus",
            Body::Neptune => "Neptune",
        }
    }

    /// The name in the URL hash: lower case.
    pub fn key(self) -> String {
        self.name().to_ascii_lowercase()
    }

    pub fn from_key(key: &str) -> Option<Body> {
        Body::ALL
            .into_iter()
            .find(|b| b.name().eq_ignore_ascii_case(key))
    }

    /// Mean radius, km (equatorial for the giants, whose flattening the
    /// view does not draw). IAU WGCCRE 2015 values.
    pub fn radius_km(self) -> f64 {
        match self {
            Body::Sun => 695_700.0,
            Body::Mercury => 2_439.4,
            Body::Venus => 6_051.8,
            Body::Earth => 6_371.0,
            Body::Moon => moon::MOON_RADIUS_KM,
            Body::Mars => 3_389.5,
            Body::Jupiter => 71_492.0,
            Body::Saturn => 60_268.0,
            Body::Uranus => 25_559.0,
            Body::Neptune => 24_764.0,
        }
    }

    pub fn radius_au(self) -> f64 {
        self.radius_km() / AU_KM
    }

    /// The planet `ephemeris` computes for this body, if any.
    pub fn planet(self) -> Option<Planet> {
        Some(match self {
            Body::Mercury => Planet::Mercury,
            Body::Venus => Planet::Venus,
            Body::Earth => Planet::Earth,
            Body::Mars => Planet::Mars,
            Body::Jupiter => Planet::Jupiter,
            Body::Saturn => Planet::Saturn,
            Body::Uranus => Planet::Uranus,
            Body::Neptune => Planet::Neptune,
            Body::Sun | Body::Moon => return None,
        })
    }

    /// The closest the camera comes to the body's center, au: a little
    /// above the surface.
    pub fn min_dist(self) -> f64 {
        1.25 * self.radius_au()
    }

    /// Where a fly-to stops: the body fills about a sixth of the field (the
    /// Sun: the home view's distance).
    pub fn arrival_dist(self) -> f64 {
        match self {
            Body::Sun => HOME.3,
            _ => 8.0 * self.radius_au(),
        }
    }
}

/// `ephemeris`' ecliptic axes (x to the equinox, z to the pole) to solar
/// space.
pub fn ecl_to_solar([x, y, z]: [f64; 3]) -> [f64; 3] {
    [y, z, x]
}

/// Solar space to the sky view's equatorial space (both J2000): a turn of
/// the obliquity about the equinox direction, +Z.
pub const SOLAR_TO_EQ: Mat3 = {
    // cos and sin of 23.43927944 degrees (const fn has no trigonometry);
    // a test checks them against OBLIQUITY_J2000_DEG.
    const C: f64 = 0.917_482_143_096_097_4;
    const S: f64 = 0.397_776_969_041_436_7;
    [[C, -S, 0.0], [S, C, 0.0], [0.0, 0.0, 1.0]]
};

/// `ephemeris`' equatorial axes (x to the equinox, z to the celestial pole)
/// to solar space.
fn equatorial_to_solar([x, y, z]: [f64; 3]) -> [f64; 3] {
    // To sky equatorial space (y, z, x), then back through the obliquity.
    mul(&transpose(&SOLAR_TO_EQ), [y, z, x])
}

/// The Moon's position relative to the Earth, au, solar space axes.
fn moon_offset(t: f64) -> Option<[f64; 3]> {
    let g = moon::geocentric_equatorial(t)?;
    Some(equatorial_to_solar(g).map(|v| v / AU_KM))
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    a.map(|v| v * k)
}

pub fn length(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

/// Heliocentric position of a body, au, solar space, at `t` Julian centuries
/// of TT from J2000. `None` outside the ephemerides' span (3000 BC to 3000
/// AD for the planets; the Moon's series is used within 30 centuries).
pub fn position(body: Body, t: f64) -> Option<[f64; 3]> {
    if let Some(p) = body.planet() {
        let bary = ecl_to_solar(ephemeris::heliocentric_ecliptic(p, t)?);
        if p != Planet::Earth {
            return Some(bary);
        }
        // The Earth sits off the barycenter, opposite the Moon.
        let m = moon_offset(t)?;
        return Some(sub(bary, scale(m, 1.0 / (1.0 + EARTH_MOON_MASS_RATIO))));
    }
    match body {
        // Only where the planets are known, so every body has the same span.
        Body::Sun => ephemeris::heliocentric_ecliptic(Planet::Earth, t).map(|_| [0.0; 3]),
        Body::Moon => Some(add(position(Body::Earth, t)?, moon_offset(t)?)),
        _ => None,
    }
}

/// A circle on screen, CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Disk {
    pub x: f64,
    pub y: f64,
    pub r: f64,
}

/// The camera, plus an optional fly-to in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolarCamera {
    /// The body orbited (during a flight, the one being flown to).
    pub target: Body,
    /// Distance from the focus to the camera, au.
    pub dist: f64,
    /// Look direction (azimuth and altitude in solar space), field of view
    /// and viewport; see the module docs.
    pub look: SkyCamera,
    pub flight: Option<Flight>,
}

impl Default for SolarCamera {
    fn default() -> Self {
        let (target, az, alt, dist) = HOME;
        SolarCamera::new(target, az, alt, dist)
    }
}

impl SolarCamera {
    pub fn new(target: Body, az: f64, alt: f64, dist: f64) -> SolarCamera {
        let mut c = SolarCamera {
            target,
            dist: HOME.3,
            look: SkyCamera::new(az, alt, FOV),
            flight: None,
        };
        c.set_dist(dist);
        c
    }

    /// Set the distance, clamped to the target's surface and [`MAX_DIST`];
    /// non-finite values keep the current one.
    pub fn set_dist(&mut self, dist: f64) {
        if dist.is_finite() {
            self.dist = dist.clamp(self.target.min_dist(), MAX_DIST);
        }
    }

    /// Unit vector the camera looks along, solar space.
    pub fn look_dir(&self) -> [f64; 3] {
        horiz_vec(self.look.az, self.look.alt)
    }

    /// The point looked at and the distance to it at time `t`: the target's
    /// center, or a point on the way during a flight.
    pub fn focus(&self, t: f64) -> Option<([f64; 3], f64)> {
        match &self.flight {
            None => Some((position(self.target, t)?, self.dist)),
            Some(f) => f.at(t),
        }
    }

    /// The camera's position at `t`, au, solar space.
    pub fn eye(&self, t: f64) -> Option<[f64; 3]> {
        let (focus, dist) = self.focus(t)?;
        Some(sub(focus, scale(self.look_dir(), dist)))
    }

    /// `p - eye` in camera space (+X right, +Y up, -Z ahead), in `f64`:
    /// the only form in which positions go to the GPU (see the module
    /// docs).
    pub fn relative(&self, eye: [f64; 3], p: [f64; 3]) -> [f64; 3] {
        mul(&transpose(&self.look.basis()), sub(p, eye))
    }

    /// Screen position (CSS pixels) of a point, seen from `eye`.
    #[cfg(test)]
    pub fn project(&self, eye: [f64; 3], p: [f64; 3]) -> Option<(f64, f64)> {
        self.look.project(sub(p, eye))
    }

    /// The outline on screen of a sphere of radius `r` (au) centered at `c`,
    /// seen from `eye`: an exact circle under the stereographic projection.
    /// `None` if the camera is inside the sphere or the sphere reaches
    /// behind the camera's back (the point opposite the view center), where
    /// the outline is no longer a bounded circle.
    pub fn disk(&self, eye: [f64; 3], c: [f64; 3], r: f64) -> Option<Disk> {
        let cam = self.relative(eye, c);
        let d = length(cam);
        if !(d.is_finite() && r.is_finite() && d > r && r >= 0.0) {
            return None;
        }
        // Angle of the center from the view direction (-Z), and the
        // sphere's angular radius.
        let a = (cam[0].hypot(cam[1])).atan2(-cam[2]);
        let rho = (r / d).asin();
        if a + rho >= PI - 1e-9 {
            return None;
        }
        // The outline's nearest and farthest points from the view center
        // lie on the great circle through it; their screen radii are
        // signed (the near one may be past the center).
        let k = self.look.half_short() / self.look.tan_quarter();
        let (r1, r2) = (k * ((a - rho) / 2.0).tan(), k * ((a + rho) / 2.0).tan());
        let side = cam[0].hypot(cam[1]);
        let (ux, uy) = if side > 0.0 {
            (cam[0] / side, cam[1] / side)
        } else {
            (1.0, 0.0)
        };
        let mid = 0.5 * (r1 + r2);
        Some(Disk {
            x: 0.5 * self.look.width + ux * mid,
            y: 0.5 * self.look.height - uy * mid,
            r: 0.5 * (r2 - r1),
        })
    }

    /// Orbit the focus by a pointer drag of `(dx, dy)` CSS pixels: the near
    /// side of the target turns with the pointer, half a turn per screen
    /// height.
    pub fn orbit(&mut self, dx: f64, dy: f64) {
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        let k = PI / (2.0 * self.look.half_short());
        let (az, alt, fov) = (self.look.az, self.look.alt, self.look.fov);
        self.look.set(az + dx * k, alt - dy * k, fov);
    }

    /// Arrow keys: orbit by 7.5 degrees per step of `dx`, `dy` (each -1, 0
    /// or 1), turning the target as a drag to the right or down would.
    pub fn nudge(&mut self, dx: f64, dy: f64) {
        let step = PI / 24.0;
        let (az, alt, fov) = (self.look.az, self.look.alt, self.look.fov);
        self.look.set(az + dx * step, alt - dy * step, fov);
    }

    /// Multiply the distance by `factor` (towards or away from the focus).
    pub fn zoom_by(&mut self, factor: f64) {
        if factor.is_finite() && factor > 0.0 {
            self.set_dist(self.dist * factor);
        }
    }

    /// Start a flight to `to`, at time `t` (the flight follows the bodies as
    /// they move). `instant` (reduced motion) jumps there. Flying to the
    /// current target, mid-flight or not, resets the distance.
    pub fn fly_to(&mut self, to: Body, t: f64, instant: bool) {
        let from = self.focus(t);
        self.target = to;
        self.flight = None;
        let d1 = to.arrival_dist();
        let (Some((p0, d0)), Some(p1), false) = (from, position(to, t), instant) else {
            self.set_dist(d1);
            return;
        };
        let gap = length(sub(p1, p0));
        self.flight = Some(Flight {
            from: p0,
            from_dist: d0,
            to,
            to_dist: d1,
            // High enough mid-way to see both ends.
            peak: d0.max(d1).max(1.2 * gap),
            s: 0.0,
        });
        self.dist = d1;
    }

    /// Advance a flight by `dt_ms`; returns whether one is still running.
    pub fn step(&mut self, dt_ms: f64) -> bool {
        let Some(f) = &mut self.flight else {
            return false;
        };
        f.s += dt_ms.max(0.0) / FLIGHT_MS;
        if f.s.is_nan() || f.s >= 1.0 {
            self.flight = None;
            return false;
        }
        true
    }
}

impl Look for SolarCamera {
    fn drag(&mut self, x0: f64, y0: f64, x1: f64, y1: f64) {
        self.orbit(x1 - x0, y1 - y0);
    }

    /// Pinch and wheel zoom go towards the focus (the anchor is ignored:
    /// the camera always looks at its target).
    fn zoom(&mut self, factor: f64, _toward: Option<(f64, f64)>) {
        self.zoom_by(factor);
    }
}

/// A fly-to: the focus slides from where the camera looked to the target
/// (which keeps moving), and the distance rises to `peak` and comes down,
/// on a log scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    /// The focus and distance when the flight began. The start point stays
    /// put; the end point is the target, wherever it is now.
    pub from: [f64; 3],
    pub from_dist: f64,
    pub to: Body,
    pub to_dist: f64,
    pub peak: f64,
    /// Progress, 0 to 1.
    pub s: f64,
}

fn ease(s: f64) -> f64 {
    let s = s.clamp(0.0, 1.0);
    s * s * (3.0 - 2.0 * s)
}

impl Flight {
    /// Focus and distance at time `t` (the scrubber's, for the target).
    pub fn at(&self, t: f64) -> Option<([f64; 3], f64)> {
        let to = position(self.to, t)?;
        let k = ease(self.s);
        let focus = add(self.from, scale(sub(to, self.from), k));
        let (l0, lp, l1) = (self.from_dist.ln(), self.peak.ln(), self.to_dist.ln());
        let ld = if self.s < 0.5 {
            l0 + (lp - l0) * ease(2.0 * self.s)
        } else {
            lp + (l1 - lp) * ease(2.0 * self.s - 1.0)
        };
        Some((focus, ld.exp()))
    }
}

/// The solar part of the URL hash:
/// `&view=solar&orbit=<target>,<az>,<alt>,<dist au>` (degrees). The target
/// and distance are where a flight in progress will end.
pub fn format_hash(cam: &SolarCamera) -> String {
    format!(
        "&view=solar&orbit={},{:.1},{:.1},{}",
        cam.target.key(),
        cam.look.az.to_degrees(),
        cam.look.alt.to_degrees(),
        format_dist_param(cam.dist)
    )
}

/// Four significant figures, without an exponent for ordinary values.
fn format_dist_param(d: f64) -> String {
    if (0.001..1000.0).contains(&d) {
        let decimals = (3 - d.log10().floor() as i32).max(0) as usize;
        let s = format!("{d:.decimals$}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        format!("{d:.3e}")
    }
}

/// The camera from `&orbit=target,az,alt,dist`, or `None` if absent or
/// malformed. The distance is clamped to the target's limits.
pub fn parse_hash_orbit(hash: &str) -> Option<SolarCamera> {
    let value = hash
        .trim_start_matches('#')
        .split('&')
        .skip(1)
        .find_map(|kv| kv.strip_prefix("orbit="))?;
    let mut parts = value.split(',');
    let target = Body::from_key(parts.next()?)?;
    let v: Vec<f64> = parts
        .map(|p| p.parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect::<Option<_>>()?;
    let [az, alt, dist] = v[..] else {
        return None;
    };
    (dist > 0.0).then(|| SolarCamera::new(target, az.to_radians(), alt.to_radians(), dist))
}

/// A distance for the HUD: km below a hundredth of an au, au above.
pub fn format_distance(au: f64) -> String {
    if !au.is_finite() {
        return "—".into();
    }
    if au < 0.01 {
        let km = (au * AU_KM).round() as i64;
        let digits = km.to_string();
        let mut out = String::new();
        for (i, ch) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(ch);
        }
        format!("{out} km")
    } else if au < 10.0 {
        format!("{au:.2} au")
    } else {
        format!("{au:.1} au")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ephemeris::centuries_tt_from_unix;

    const DEG: f64 = PI / 180.0;
    /// Mean obliquity of the ecliptic at J2000.0 (as in `ephemeris`).
    const OBLIQUITY_J2000_DEG: f64 = 23.439_279_44;
    /// 2025-01-01 00:00 UTC.
    const T0_UNIX: f64 = 1_735_689_600.0;

    fn t0() -> f64 {
        centuries_tt_from_unix(T0_UNIX).unwrap()
    }

    fn close(a: [f64; 3], b: [f64; 3], eps: f64) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < eps)
    }

    fn sized(mut cam: SolarCamera) -> SolarCamera {
        cam.look.width = 1200.0;
        cam.look.height = 800.0;
        cam
    }

    #[test]
    fn solar_space_follows_the_conventions() {
        // The equinox on +Z, longitude 90 on +X, the ecliptic pole on +Y.
        assert_eq!(ecl_to_solar([1.0, 0.0, 0.0]), [0.0, 0.0, 1.0]);
        assert_eq!(ecl_to_solar([0.0, 1.0, 0.0]), [1.0, 0.0, 0.0]);
        assert_eq!(ecl_to_solar([0.0, 0.0, 1.0]), [0.0, 1.0, 0.0]);
        // Right-handed: x cross y = z still holds after the swap.
        let (x, y) = (ecl_to_solar([1.0, 0.0, 0.0]), ecl_to_solar([0.0, 1.0, 0.0]));
        let z = [
            x[1] * y[2] - x[2] * y[1],
            x[2] * y[0] - x[0] * y[2],
            x[0] * y[1] - x[1] * y[0],
        ];
        assert_eq!(z, ecl_to_solar([0.0, 0.0, 1.0]));
    }

    #[test]
    fn solar_to_equatorial_is_the_obliquity() {
        let (s, c) = OBLIQUITY_J2000_DEG.to_radians().sin_cos();
        assert!((SOLAR_TO_EQ[0][0] - c).abs() < 1e-15);
        assert!((SOLAR_TO_EQ[1][0] - s).abs() < 1e-15);
        // The equinox stays at RA 0, Dec 0.
        assert!(close(
            mul(&SOLAR_TO_EQ, [0.0, 0.0, 1.0]),
            [0.0, 0.0, 1.0],
            1e-15
        ));
        // The north ecliptic pole is at RA 18h, Dec 66.56 degrees.
        let pole = mul(&SOLAR_TO_EQ, [0.0, 1.0, 0.0]);
        let (ra, dec) = crate::sky::vec_radec(pole);
        assert!((ra - 270.0 * DEG).abs() < 1e-12, "{}", ra / DEG);
        assert!((dec - (90.0 - OBLIQUITY_J2000_DEG) * DEG).abs() < 1e-12);
        // It agrees with ephemeris' ecliptic-to-equatorial rotation.
        let v = [0.3, -0.5, 0.8];
        let eq = ephemeris::ecliptic_to_equatorial(v);
        let want = [eq[1], eq[2], eq[0]]; // sky equatorial axes
        assert!(close(mul(&SOLAR_TO_EQ, ecl_to_solar(v)), want, 1e-15));
        assert!(close(equatorial_to_solar(eq), ecl_to_solar(v), 1e-15));
    }

    #[test]
    fn positions_are_where_horizons_has_them() {
        // JPL Horizons (DE441), 2025-01-01 00:00 TDB, heliocentric, J2000
        // ecliptic, au: the geocenter (399), the Moon (301) and the
        // Earth-Moon barycenter (3). Query: EPHEM_TYPE=VECTORS,
        // CENTER='500@10', REF_PLANE=ECLIPTIC, fetched 2026-11-20.
        let t = crate::ephemeris::centuries_from_jd(2_460_676.5);
        let earth = ecl_to_solar([
            -1.786_834_409_731_047E-1,
            9.669_827_953_774_551E-1,
            -5.109_423_915_082_682E-5,
        ]);
        let moon = ecl_to_solar([
            -1.776_670_337_217_189E-1,
            9.646_511_868_801_392E-1,
            -2.560_710_414_773_878E-4,
        ]);
        let bary = ecl_to_solar([
            -1.786_710_910_310_161E-1,
            9.669_544_649_716_304E-1,
            -5.358_482_708_673_672E-5,
        ]);
        let km = |a: [f64; 3], b: [f64; 3]| length(sub(a, b)) * AU_KM;
        let got_e = position(Body::Earth, t).unwrap();
        let got_m = position(Body::Moon, t).unwrap();
        // The barycenter's elements are good to some thousands of km.
        let got_bary = ecl_to_solar(ephemeris::heliocentric_ecliptic(Planet::Earth, t).unwrap());
        let bary_err = km(got_bary, bary);
        assert!(bary_err < 20_000.0, "{bary_err}");
        // The Earth's offset from the barycenter (about 4,700 km, away from
        // the Moon) is right to a few km, and the Moon about the Earth to
        // a few hundred.
        let off = km(sub(got_e, got_bary), sub(earth, bary));
        assert!(off < 10.0, "{off}");
        let moon_err = km(sub(got_m, got_e), sub(moon, earth));
        assert!(moon_err < 300.0, "{moon_err}");
        // So the Earth and Moon are only as far off as the barycenter.
        assert!(km(got_e, earth) < bary_err + 10.0);
    }

    #[test]
    fn positions_fail_cleanly() {
        for b in Body::ALL {
            assert!(position(b, f64::NAN).is_none(), "{b:?}");
            assert!(position(b, 1e9).is_none(), "{b:?}");
        }
        assert_eq!(position(Body::Sun, 0.0), Some([0.0; 3]));
    }

    #[test]
    fn the_target_is_at_the_center_and_round() {
        let t = t0();
        for b in Body::ALL {
            let mut cam = sized(SolarCamera::new(b, 1.0, 0.4, 0.0));
            cam.set_dist(b.arrival_dist());
            let eye = cam.eye(t).unwrap();
            let p = position(b, t).unwrap();
            let (x, y) = cam.project(eye, p).unwrap();
            assert!(
                (x - 600.0).abs() < 1e-6 && (y - 400.0).abs() < 1e-6,
                "{b:?}"
            );
            let d = cam.disk(eye, p, b.radius_au()).unwrap();
            assert!((d.x - 600.0).abs() < 1e-6 && (d.y - 400.0).abs() < 1e-6);
            // Angular radius asin(1/8) under the stereographic scale.
            let want = 400.0 * ((1.0f64 / 8.0).asin() / 2.0).tan() / (FOV / 4.0).tan();
            let want_sun =
                400.0 * ((b.radius_au() / HOME.3).asin() / 2.0).tan() / (FOV / 4.0).tan();
            let want = if b == Body::Sun { want_sun } else { want };
            assert!((d.r - want).abs() < 1e-6 * want, "{b:?} {} {want}", d.r);
        }
    }

    #[test]
    fn disks_are_the_projected_outline() {
        // An off-center sphere: every point of its true outline projects
        // onto the circle `disk` returns.
        let cam = sized(SolarCamera::new(Body::Sun, 0.3, 0.2, 2.0));
        let eye = cam.eye(0.0).unwrap();
        let fwd = cam.look_dir();
        // A sphere ahead and to the side.
        let c = add(eye, add(scale(fwd, 1.0), [0.25, -0.1, 0.05]));
        let r = 0.08;
        let disk = cam.disk(eye, c, r).unwrap();
        let rel = sub(c, eye);
        let d = length(rel);
        let u = scale(rel, 1.0 / d);
        // Two vectors perpendicular to u.
        let a = {
            let t = [u[1], -u[0], 0.0];
            scale(t, 1.0 / length(t))
        };
        let b = [
            u[1] * a[2] - u[2] * a[1],
            u[2] * a[0] - u[0] * a[2],
            u[0] * a[1] - u[1] * a[0],
        ];
        let rho = (r / d).asin();
        for i in 0..36 {
            let phi = i as f64 * 10.0 * DEG;
            let dir = add(
                scale(u, rho.cos()),
                scale(add(scale(a, phi.cos()), scale(b, phi.sin())), rho.sin()),
            );
            let (x, y) = cam.look.project(dir).unwrap();
            let off = (x - disk.x).hypot(y - disk.y);
            assert!(
                (off - disk.r).abs() < 1e-6 * disk.r,
                "{i}: {off} vs {}",
                disk.r
            );
        }
        // Inside the sphere, or reaching behind the camera: no circle.
        assert!(cam.disk(eye, eye, 0.1).is_none());
        let behind = sub(eye, scale(fwd, 1.0));
        assert!(cam.disk(eye, behind, 0.1).is_none());
        assert!(cam.disk(eye, c, f64::NAN).is_none());
    }

    #[test]
    fn camera_relative_coordinates_keep_precision() {
        // A camera 2,000 km from Neptune's center (30 au out).
        let t = t0();
        let mut cam = sized(SolarCamera::new(Body::Neptune, 2.0, 0.3, 1.0));
        cam.dist = 30_000.0 / AU_KM;
        let eye = cam.eye(t).unwrap();
        let p = position(Body::Neptune, t).unwrap();
        // The camera-relative position, cast to f32 for the GPU, is off by
        // a few millimetres per kilometre.
        let rel = cam.relative(eye, p);
        let rel32 = rel.map(|v| v as f32 as f64);
        let err = length(sub(rel, rel32)) * AU_KM;
        assert!(err < 1e-3, "{err} km");
        assert!((length(rel) * AU_KM - 30_000.0).abs() < 1e-6);
        // Heliocentric positions cast to f32 first would put Neptune
        // hundreds of km off: why nothing heliocentric reaches the GPU.
        let p32 = p.map(|v| v as f32 as f64);
        let e32 = eye.map(|v| v as f32 as f64);
        let naive = length(sub(sub(p32, e32), sub(p, eye))) * AU_KM;
        assert!(naive > 50.0, "{naive} km");
    }

    #[test]
    fn orbiting_turns_the_near_side_with_the_pointer() {
        let t = t0();
        let mut cam = sized(SolarCamera::new(Body::Earth, 0.5, 0.2, 0.0));
        cam.set_dist(0.001);
        let p = position(Body::Earth, t).unwrap();
        // The point of the Earth's surface nearest the camera.
        let near = |c: &SolarCamera| sub(p, scale(c.look_dir(), Body::Earth.radius_au()));
        let pt = near(&cam);
        let (x0, y0) = cam.project(cam.eye(t).unwrap(), pt).unwrap();
        cam.orbit(30.0, 20.0);
        let (x1, y1) = cam.project(cam.eye(t).unwrap(), pt).unwrap();
        assert!(x1 > x0 + 1.0, "right: {x0} -> {x1}");
        assert!(y1 > y0 + 1.0, "down: {y0} -> {y1}");
        // The target stays centered, and the distance is unchanged.
        let (cx, cy) = cam.project(cam.eye(t).unwrap(), p).unwrap();
        assert!((cx - 600.0).abs() < 1e-6 && (cy - 400.0).abs() < 1e-6);
        assert_eq!(cam.dist, 0.001);
        // The altitude stops short of the poles.
        cam.orbit(0.0, 1e6);
        assert!(cam.look.alt <= crate::sky::ALT_LIMIT);
        cam.orbit(f64::NAN, 0.0);
        assert!(cam.look.az.is_finite());
        // Arrow keys turn the same way as a drag.
        let mut cam = sized(SolarCamera::new(Body::Earth, 0.5, 0.2, 0.001));
        let (x0, y0) = cam.project(cam.eye(t).unwrap(), near(&cam)).unwrap();
        let pt = near(&cam);
        cam.nudge(1.0, 1.0);
        let (x1, y1) = cam.project(cam.eye(t).unwrap(), pt).unwrap();
        assert!(x1 > x0 + 1.0 && y1 > y0 + 1.0, "{x0},{y0} -> {x1},{y1}");
    }

    #[test]
    fn zoom_stays_between_the_surface_and_the_edge() {
        let mut cam = SolarCamera::new(Body::Jupiter, 0.0, 0.0, 1.0);
        cam.zoom_by(1e-9);
        assert_eq!(cam.dist, Body::Jupiter.min_dist());
        cam.zoom_by(1e12);
        assert_eq!(cam.dist, MAX_DIST);
        cam.zoom_by(0.5);
        assert_eq!(cam.dist, MAX_DIST / 2.0);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            cam.zoom_by(bad);
            assert_eq!(cam.dist, MAX_DIST / 2.0);
        }
        Look::zoom(&mut cam, 2.0, Some((1.0, 2.0)));
        assert_eq!(cam.dist, MAX_DIST);
    }

    #[test]
    fn flights_start_where_the_view_was_and_end_on_the_target() {
        let t = t0();
        let mut cam = SolarCamera::default();
        let before = cam.eye(t).unwrap();
        cam.fly_to(Body::Mars, t, false);
        assert_eq!(cam.target, Body::Mars);
        assert!(
            close(cam.eye(t).unwrap(), before, 1e-12),
            "no jump at the start"
        );
        let mars = position(Body::Mars, t).unwrap();
        let gap = length(mars);
        let mut most: f64 = 0.0;
        let mut least = f64::INFINITY;
        let mut steps = 0;
        while cam.step(16.0) {
            let (_, d) = cam.focus(t).unwrap();
            most = most.max(d);
            least = least.min(d);
            steps += 1;
        }
        assert!(steps > 100, "{steps}");
        // It rose high enough to see both ends, and never went below the
        // arrival distance.
        assert!(most > gap, "{most} vs {gap}");
        assert!(least >= Body::Mars.arrival_dist() * (1.0 - 1e-9));
        let (focus, d) = cam.focus(t).unwrap();
        assert!(close(focus, mars, 1e-15));
        assert_eq!(d, Body::Mars.arrival_dist());
        assert!(cam.flight.is_none());
        // Reduced motion jumps.
        cam.fly_to(Body::Saturn, t, true);
        assert!(cam.flight.is_none());
        assert_eq!(cam.target, Body::Saturn);
        assert_eq!(cam.dist, Body::Saturn.arrival_dist());
    }

    #[test]
    fn a_flight_follows_a_moving_target() {
        // The time runs on during a flight: the end is where the target is
        // at the end, not where it was.
        let t = t0();
        let mut cam = SolarCamera::new(Body::Earth, 0.0, 0.3, 0.001);
        cam.fly_to(Body::Moon, t, false);
        while cam.step(100.0) {}
        let later = t + 1.0 / 36_525.0; // a day
        let (focus, _) = cam.focus(later).unwrap();
        assert!(close(focus, position(Body::Moon, later).unwrap(), 1e-15));
    }

    #[test]
    fn the_hash_round_trips() {
        let mut cam = SolarCamera::new(Body::Saturn, 1.2, -0.4, 0.0);
        cam.set_dist(0.004);
        let hash = format!("#40,-74,800{}", format_hash(&cam));
        assert_eq!(hash, "#40,-74,800&view=solar&orbit=saturn,68.8,-22.9,0.004");
        let back = parse_hash_orbit(&hash).unwrap();
        assert_eq!(back.target, Body::Saturn);
        assert!((back.look.az - 1.2).abs() < 0.001);
        assert!((back.look.alt + 0.4).abs() < 0.001);
        assert_eq!(back.dist, 0.004);
        // Tiny and huge distances use an exponent; both parse.
        cam.target = Body::Moon;
        cam.dist = 2.3456e-5;
        let h = format_hash(&cam);
        assert!(h.ends_with(",2.346e-5"), "{h}");
        assert!((parse_hash_orbit(&format!("#0,0,1{h}")).unwrap().dist - 2.346e-5).abs() < 1e-12);
        // Out of range distances are clamped to the target.
        let near = parse_hash_orbit("#0,0,1&orbit=earth,0,0,1e-12").unwrap();
        assert_eq!(near.dist, Body::Earth.min_dist());
        // Malformed.
        for bad in [
            "#0,0,1",
            "#0,0,1&orbit=pluto,0,0,1",
            "#0,0,1&orbit=earth,0,0",
            "#0,0,1&orbit=earth,0,0,1,2",
            "#0,0,1&orbit=earth,x,0,1",
            "#0,0,1&orbit=earth,0,0,-1",
            "#0,0,1&orbit=earth,0,0,0",
            "#0,0,1&orbit=earth,NaN,0,1",
            "#0,0,1&orbit=earth,0,0,inf",
        ] {
            assert!(parse_hash_orbit(bad).is_none(), "{bad}");
        }
        // Any case of the name.
        assert_eq!(
            parse_hash_orbit("#0,0,1&orbit=JUPITER,0,0,1")
                .unwrap()
                .target,
            Body::Jupiter
        );
    }

    #[test]
    fn distances_read_naturally() {
        assert_eq!(format_distance(42_600.0 / AU_KM), "42,600 km");
        assert_eq!(format_distance(800.0 / AU_KM), "800 km");
        assert_eq!(format_distance(1_234_567.0 / AU_KM), "1,234,567 km");
        assert_eq!(format_distance(3.5), "3.50 au");
        assert_eq!(format_distance(45.25), "45.2 au");
        assert_eq!(format_distance(f64::NAN), "—");
    }

    #[test]
    fn bodies_have_names_and_keys() {
        for b in Body::ALL {
            assert_eq!(Body::from_key(&b.key()), Some(b));
            assert!(b.radius_km() > 1000.0);
            assert!(b.min_dist() < b.arrival_dist());
        }
        assert_eq!(Body::from_key("pluto"), None);
        assert_eq!(Body::Earth.planet(), Some(Planet::Earth));
        assert_eq!(Body::Moon.planet(), None);
    }
}
