//! The sky view: an observer at the center of the celestial sphere, looking
//! out. Browser-free (tested natively); `sky_render.rs` draws it and
//! `src/shaders/sky.frag` repeats the projection math.
//!
//! Coordinate conventions (see AGENTS.md):
//!
//! - **Equatorial space** (J2000 right ascension and declination) mirrors
//!   Earth's world space: +Y is the north celestial pole, RA 0h lies on +Z
//!   and RA 6h on +X. A star's unit vector is
//!   `(cos dec sin ra, sin dec, cos dec cos ra)`.
//! - **Horizontal space** (azimuth from north through east, altitude above
//!   the horizon) for the observer: +Y is the zenith, north is on -Z and east
//!   on +X, so `(cos alt sin az, sin alt, -cos alt cos az)`. North on -Z
//!   makes the frame right-handed like the equatorial one, so the change
//!   between them is a rotation ([`eq_to_horizon`]).
//! - **Camera space**: the camera looks down -Z with +Y up, as on Earth. The
//!   view is an azimuth and altitude of the center plus a field of view;
//!   there is no roll, so the zenith is always up the screen.
//! - **Projection**: stereographic about the view direction, as planetarium
//!   programs use: it keeps shapes round and reaches past 180 degrees. A
//!   direction at angle `a` from the center lands at radius
//!   `tan(a / 2) / tan(fov / 4)` in units of half the shorter screen axis, so
//!   the field of view spans the shorter axis, as the Earth camera's does.
//!
//! The observer stands at the Earth view's center (`Camera::lat`/`lon`) at
//! the time of the time scrubber. Catalog positions are J2000 and are not
//! precessed to the date; the error grows by about 0.014 degrees a year
//! (0.36 degrees by 2026), which is below what the view resolves at its
//! default field.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// Row-major 3x3 matrix.
pub type Mat3 = [[f64; 3]; 3];

/// Narrowest and widest field of view.
pub const FOV_MIN: f64 = 0.5 * PI / 180.0;
pub const FOV_MAX: f64 = 150.0 * PI / 180.0;
/// Highest and lowest view altitude; the zenith itself has no azimuth, so
/// the view stops just short of it.
pub const ALT_LIMIT: f64 = 89.5 * PI / 180.0;
/// The view a reset returns to: due south, a third of the way up, wide.
pub const HOME: (f64, f64, f64) = (PI, 30.0 * PI / 180.0, 100.0 * PI / 180.0);
/// Arrow keys turn the view by this fraction of the field of view.
pub const NUDGE: f64 = 0.15;

pub fn mul(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

pub fn transpose(m: &Mat3) -> Mat3 {
    let mut t = [[0.0; 3]; 3];
    for (i, row) in m.iter().enumerate() {
        for (j, v) in row.iter().enumerate() {
            t[j][i] = *v;
        }
    }
    t
}

pub fn matmul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// Column-major `f32` copy for `uniformMatrix3fv`.
pub fn to_gl(m: &Mat3) -> [f32; 9] {
    let mut out = [0.0; 9];
    for (c, col) in out.chunks_mut(3).enumerate() {
        for (r, v) in col.iter_mut().enumerate() {
            *v = m[r][c] as f32;
        }
    }
    out
}

/// Wrap an angle to `[0, 2 pi)`.
pub fn wrap_tau(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w >= TAU {
        0.0
    } else {
        w
    }
}

/// Wrap an angle to `(-pi, pi]`.
fn wrap_pi(a: f64) -> f64 {
    let w = (a + PI).rem_euclid(TAU) - PI;
    if w <= -PI {
        PI
    } else {
        w
    }
}

/// Unit vector of right ascension and declination (radians).
pub fn radec_vec(ra: f64, dec: f64) -> [f64; 3] {
    [dec.cos() * ra.sin(), dec.sin(), dec.cos() * ra.cos()]
}

/// Right ascension in `[0, 2 pi)` and declination of a direction.
pub fn vec_radec(v: [f64; 3]) -> (f64, f64) {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (
        wrap_tau(v[0].atan2(v[2])),
        (v[1] / len).clamp(-1.0, 1.0).asin(),
    )
}

/// Unit vector of azimuth (from north through east) and altitude (radians).
pub fn horiz_vec(az: f64, alt: f64) -> [f64; 3] {
    [alt.cos() * az.sin(), alt.sin(), -alt.cos() * az.cos()]
}

/// Azimuth in `[0, 2 pi)` and altitude of a horizontal direction.
pub fn vec_horiz(v: [f64; 3]) -> (f64, f64) {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (
        wrap_tau(v[0].atan2(-v[2])),
        (v[1] / len).clamp(-1.0, 1.0).asin(),
    )
}

/// Local sidereal time (radians, `[0, 2 pi)`) at east longitude `lon`.
pub fn local_sidereal_time(unix_secs: f64, lon: f64) -> f64 {
    wrap_tau(crate::sun::gmst(unix_secs) + lon)
}

/// Rotation from equatorial to horizontal space for an observer at
/// latitude `lat` when the local sidereal time is `lst`.
///
/// First turn about the pole so the observer's meridian (RA = LST) lies on
/// +Z; east (hour angle -6h) is then on +X. Then tilt about the east axis:
/// the pole goes to the north at altitude `lat`, `(0, sin lat, -cos lat)`,
/// and the equator on the meridian to the south at altitude `90 - lat`,
/// `(0, cos lat, sin lat)`.
pub fn eq_to_horizon(lat: f64, lst: f64) -> Mat3 {
    let (s, c) = (-lst).sin_cos();
    // About +Y by -LST: RA a goes to a - LST.
    let spin: Mat3 = [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]];
    let (sl, cl) = lat.sin_cos();
    let tilt: Mat3 = [[1.0, 0.0, 0.0], [0.0, sl, cl], [0.0, -cl, sl]];
    matmul(&tilt, &spin)
}

/// Where an observer stands and when.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Observer {
    /// Geographic latitude and east longitude, radians.
    pub lat: f64,
    pub lon: f64,
    pub unix_secs: f64,
}

impl Observer {
    pub fn eq_to_horizon(&self) -> Mat3 {
        eq_to_horizon(self.lat, local_sidereal_time(self.unix_secs, self.lon))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyCamera {
    /// Azimuth (from north through east) and altitude of the view center,
    /// radians.
    pub az: f64,
    pub alt: f64,
    /// Field of view across the shorter screen axis, radians.
    pub fov: f64,
    /// Viewport size in CSS pixels.
    pub width: f64,
    pub height: f64,
}

impl Default for SkyCamera {
    fn default() -> Self {
        SkyCamera::new(HOME.0, HOME.1, HOME.2)
    }
}

impl SkyCamera {
    pub fn new(az: f64, alt: f64, fov: f64) -> SkyCamera {
        let mut c = SkyCamera {
            az: 0.0,
            alt: 0.0,
            fov: HOME.2,
            width: 800.0,
            height: 600.0,
        };
        c.set(az, alt, fov);
        c
    }

    /// Set the view, wrapping the azimuth and clamping the altitude and
    /// field; non-finite values keep the current ones.
    pub fn set(&mut self, az: f64, alt: f64, fov: f64) {
        if az.is_finite() {
            self.az = wrap_tau(az);
        }
        if alt.is_finite() {
            self.alt = alt.clamp(-ALT_LIMIT, ALT_LIMIT);
        }
        if fov.is_finite() {
            self.fov = fov.clamp(FOV_MIN, FOV_MAX);
        }
    }

    /// Camera-to-horizontal rotation: its columns are the screen's right
    /// and up and the backward direction, in horizontal space.
    pub fn basis(&self) -> Mat3 {
        let f = horiz_vec(self.az, self.alt);
        let r = [self.az.cos(), 0.0, self.az.sin()];
        let u = [
            r[1] * f[2] - r[2] * f[1],
            r[2] * f[0] - r[0] * f[2],
            r[0] * f[1] - r[1] * f[0],
        ];
        [
            [r[0], u[0], -f[0]],
            [r[1], u[1], -f[1]],
            [r[2], u[2], -f[2]],
        ]
    }

    /// Half the shorter viewport axis, CSS pixels.
    pub fn half_short(&self) -> f64 {
        0.5 * self.width.min(self.height).max(1.0)
    }

    /// `tan(fov / 4)`: the stereographic radius at the edge of the field.
    pub fn tan_quarter(&self) -> f64 {
        (self.fov / 4.0).tan()
    }

    /// Screen position (CSS pixels) of a horizontal direction, or `None`
    /// for the point straight behind the view (and NaN input). Points can
    /// land off screen.
    pub fn project(&self, h: [f64; 3]) -> Option<(f64, f64)> {
        let cam = mul(&transpose(&self.basis()), h);
        let len = (cam[0] * cam[0] + cam[1] * cam[1] + cam[2] * cam[2]).sqrt();
        if len.is_nan() || len <= 0.0 {
            return None;
        }
        let d = 1.0 - cam[2] / len;
        if d < 1e-9 {
            return None;
        }
        let k = self.half_short() / (self.tan_quarter() * d * len);
        Some((
            0.5 * self.width + cam[0] * k,
            0.5 * self.height - cam[1] * k,
        ))
    }

    /// Horizontal direction under a screen point (every point has one).
    pub fn pick(&self, x: f64, y: f64) -> Option<[f64; 3]> {
        if !(x.is_finite() && y.is_finite()) {
            return None;
        }
        let t = self.tan_quarter() / self.half_short();
        let (px, py) = ((x - 0.5 * self.width) * t, (0.5 * self.height - y) * t);
        let t2 = px * px + py * py;
        let cam = [2.0 * px, 2.0 * py, -(1.0 - t2)];
        let cam = cam.map(|v| v / (1.0 + t2));
        Some(mul(&self.basis(), cam))
    }

    /// Turn the view so the sky under `(x0, y0)` follows the pointer to
    /// `(x1, y1)`: the azimuth and altitude change by the difference between
    /// the two picked directions.
    pub fn drag(&mut self, x0: f64, y0: f64, x1: f64, y1: f64) {
        let (Some(a), Some(b)) = (self.pick(x0, y0), self.pick(x1, y1)) else {
            return;
        };
        let ((az0, alt0), (az1, alt1)) = (vec_horiz(a), vec_horiz(b));
        // Near the zenith a small move swings the azimuth a lot; cap it.
        let daz = wrap_pi(az0 - az1).clamp(-FRAC_PI_2, FRAC_PI_2);
        self.set(self.az + daz, self.alt + (alt0 - alt1), self.fov);
    }

    /// Multiply the field of view by `factor`, keeping the sky under
    /// `toward` (CSS pixels) in place, or the center if `None`.
    pub fn zoom(&mut self, factor: f64, toward: Option<(f64, f64)>) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let anchor = toward.and_then(|(x, y)| Some((x, y, self.pick(x, y)?)));
        self.set(self.az, self.alt, self.fov * factor);
        if let Some((x, y, want)) = anchor {
            // A few passes of "move what is now under the point back to it";
            // each one cuts the error by about ten.
            for _ in 0..4 {
                let Some((sx, sy)) = self.project(want) else {
                    return;
                };
                self.drag(sx, sy, x, y);
            }
        }
    }

    /// Arrow-key turn: `dx` and `dy` in steps of [`NUDGE`] times the field.
    pub fn nudge(&mut self, dx: f64, dy: f64) {
        let step = NUDGE * self.fov;
        let daz = dx * step / self.alt.cos().max(0.25);
        self.set(self.az + daz, self.alt + dy * step, self.fov);
    }
}

/// What [`Gestures`] steers: the sky camera, or the solar system's.
pub trait Look: Copy + PartialEq {
    /// The pointer moved from `(x0, y0)` to `(x1, y1)` (CSS pixels).
    fn drag(&mut self, x0: f64, y0: f64, x1: f64, y1: f64);
    /// Zoom by `factor` (below 1 is in), about `toward` where that applies.
    fn zoom(&mut self, factor: f64, toward: Option<(f64, f64)>);
}

impl Look for SkyCamera {
    fn drag(&mut self, x0: f64, y0: f64, x1: f64, y1: f64) {
        SkyCamera::drag(self, x0, y0, x1, y1);
    }
    fn zoom(&mut self, factor: f64, toward: Option<(f64, f64)>) {
        SkyCamera::zoom(self, factor, toward);
    }
}

/// Compass points on the horizon that are on screen, with their positions
/// (CSS pixels), for labels just above the horizon line.
pub fn horizon_labels(cam: &SkyCamera) -> Vec<(f64, f64, &'static str)> {
    (0..8)
        .filter_map(|i| {
            let az = i as f64 * TAU / 8.0;
            let (x, y) = cam.project(horiz_vec(az, 0.0))?;
            let inside =
                (-20.0..cam.width + 20.0).contains(&x) && (-20.0..cam.height + 20.0).contains(&y);
            inside.then_some((x, y, compass_point(az)))
        })
        .collect()
}

/// Spacing of the RA/Dec grid for a field of view: about six lines across,
/// on a step that divides both 90 and 360 degrees.
pub fn grid_step(fov: f64) -> f64 {
    const STEPS: [f64; 6] = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0];
    let want = fov.to_degrees() / 6.0;
    let step = STEPS.iter().copied().find(|s| *s >= want).unwrap_or(30.0);
    step.to_radians()
}

/// Zoom factor for a wheel event, as on the globe: `mode` is the DOM
/// `deltaMode`, and a trackpad pinch arrives as ctrl+wheel.
pub fn wheel_factor(dy: f64, mode: u32, ctrl: bool) -> f64 {
    let scale = match mode {
        1 => 16.0,
        2 => 400.0,
        _ => 1.0,
    };
    let k = if ctrl { 0.01 } else { 0.0018 };
    let f = (dy * scale * k).clamp(-1.0, 1.0).exp();
    if f.is_finite() {
        f
    } else {
        1.0
    }
}

/// Mouse and touch gestures for the sky: one pointer drags the view, two
/// pinch to zoom about their midpoint and drag with it.
#[derive(Debug, Default)]
pub struct Gestures {
    pointers: Vec<(i32, f64, f64)>,
}

impl Gestures {
    pub fn down(&mut self, id: i32, x: f64, y: f64) {
        if !(x.is_finite() && y.is_finite()) {
            return;
        }
        self.pointers.retain(|p| p.0 != id);
        if self.pointers.len() < 2 {
            self.pointers.push((id, x, y));
        }
    }

    pub fn up(&mut self, id: i32) {
        self.pointers.retain(|p| p.0 != id);
    }

    pub fn active(&self) -> bool {
        !self.pointers.is_empty()
    }

    /// A pointer moved; returns whether the camera changed.
    pub fn moved<C: Look>(&mut self, cam: &mut C, id: i32, x: f64, y: f64) -> bool {
        if !(x.is_finite() && y.is_finite()) {
            return false;
        }
        let Some(i) = self.pointers.iter().position(|p| p.0 == id) else {
            return false;
        };
        let before = *cam;
        let old = self.pointers.clone();
        self.pointers[i] = (id, x, y);
        match (old.as_slice(), self.pointers.as_slice()) {
            ([(_, x0, y0)], [(_, x1, y1)]) => cam.drag(*x0, *y0, *x1, *y1),
            ([a0, b0], [a1, b1]) => {
                let mid = |a: &(i32, f64, f64), b: &(i32, f64, f64)| {
                    ((a.1 + b.1) / 2.0, (a.2 + b.2) / 2.0)
                };
                let dist = |a: &(i32, f64, f64), b: &(i32, f64, f64)| (a.1 - b.1).hypot(a.2 - b.2);
                let (m0, m1) = (mid(a0, b0), mid(a1, b1));
                cam.drag(m0.0, m0.1, m1.0, m1.1);
                let (d0, d1) = (dist(a0, b0), dist(a1, b1));
                if d0 > 1.0 && d1 > 1.0 {
                    cam.zoom(d0 / d1, Some(m1));
                }
            }
            _ => {}
        }
        *cam != before
    }
}

/// The sky part of the URL hash: `&view=sky&look=<az>,<alt>,<fov>` in
/// degrees. The Earth part (`#lat,lon,...`) gives the observer.
pub fn format_hash(cam: &SkyCamera) -> String {
    format!(
        "&view=sky&look={:.1},{:.1},{}",
        cam.az.to_degrees(),
        cam.alt.to_degrees(),
        format_fov(cam.fov.to_degrees())
    )
}

fn format_fov(deg: f64) -> String {
    if deg < 10.0 {
        format!("{deg:.2}")
    } else {
        format!("{deg:.1}")
    }
}

fn hash_param<'a>(hash: &'a str, key: &str) -> Option<&'a str> {
    hash.trim_start_matches('#')
        .split('&')
        .skip(1)
        .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

/// The sky camera from `&look=az,alt,fov` (degrees), or `None` if absent or
/// malformed.
pub fn parse_hash_look(hash: &str) -> Option<SkyCamera> {
    let v: Vec<f64> = hash_param(hash, "look")?
        .split(',')
        .map(|p| p.parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect::<Option<_>>()?;
    let [az, alt, fov] = v[..] else {
        return None;
    };
    Some(SkyCamera::new(
        az.to_radians(),
        alt.to_radians(),
        fov.to_radians(),
    ))
}

/// `5h 35m` style right ascension.
pub fn format_ra(ra: f64) -> String {
    let minutes = (wrap_tau(ra) / TAU * 24.0 * 60.0).round() as i64 % (24 * 60);
    format!("{}h {:02}m", minutes / 60, minutes % 60)
}

/// The HUD text for a direction: right ascension and declination.
pub fn format_radec(ra: f64, dec: f64) -> String {
    format!("RA {}, Dec {:+.1}°", format_ra(ra), dec.to_degrees())
}

/// Compass name for an azimuth (eight points).
pub fn compass_point(az: f64) -> &'static str {
    const NAMES: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    NAMES[((wrap_tau(az) / TAU * 8.0).round() as usize) % 8]
}

/// The HUD text for the view center: azimuth (with compass point) and
/// altitude.
pub fn format_look(cam: &SkyCamera) -> String {
    format!(
        "Az {:.0}° {}, Alt {:.0}°",
        cam.az.to_degrees(),
        compass_point(cam.az),
        cam.alt.to_degrees()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEG: f64 = PI / 180.0;

    fn close(a: [f64; 3], b: [f64; 3], eps: f64) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < eps)
    }

    fn angle(a: [f64; 3], b: [f64; 3]) -> f64 {
        let d: f64 = (0..3).map(|i| a[i] * b[i]).sum();
        d.clamp(-1.0, 1.0).acos()
    }

    #[test]
    fn spaces_follow_the_conventions() {
        assert!(close(radec_vec(0.0, 0.0), [0.0, 0.0, 1.0], 1e-12));
        assert!(close(radec_vec(90.0 * DEG, 0.0), [1.0, 0.0, 0.0], 1e-12));
        assert!(close(radec_vec(1.0, FRAC_PI_2), [0.0, 1.0, 0.0], 1e-12));
        assert!(close(horiz_vec(0.0, 0.0), [0.0, 0.0, -1.0], 1e-12), "north");
        assert!(
            close(horiz_vec(90.0 * DEG, 0.0), [1.0, 0.0, 0.0], 1e-12),
            "east"
        );
        assert!(
            close(horiz_vec(3.0, FRAC_PI_2), [0.0, 1.0, 0.0], 1e-12),
            "zenith"
        );
    }

    #[test]
    fn angles_round_trip() {
        for &(a, b) in &[
            (0.0, 0.0),
            (1.0, 0.5),
            (5.9, -1.2),
            (3.2, 1.5),
            (0.001, -0.3),
        ] {
            let (ra, dec) = vec_radec(radec_vec(a, b));
            assert!((ra - a).abs() < 1e-12 && (dec - b).abs() < 1e-12, "{a} {b}");
            let (az, alt) = vec_horiz(horiz_vec(a, b));
            assert!((az - a).abs() < 1e-12 && (alt - b).abs() < 1e-12, "{a} {b}");
        }
        assert_eq!(wrap_tau(TAU), 0.0);
        assert_eq!(wrap_tau(-1e-18), 0.0);
    }

    /// Altitude and azimuth from the textbook formulas (azimuth from north
    /// through east).
    fn classical(ra: f64, dec: f64, lat: f64, lst: f64) -> (f64, f64) {
        let h = lst - ra;
        let alt = (dec.sin() * lat.sin() + dec.cos() * lat.cos() * h.cos()).asin();
        let az =
            (-dec.cos() * h.sin()).atan2(dec.sin() * lat.cos() - dec.cos() * lat.sin() * h.cos());
        (wrap_tau(az), alt)
    }

    #[test]
    fn eq_to_horizon_matches_the_textbook_formulas() {
        for &(ra, dec, lat, lst) in &[
            (0.3, 0.2, 0.7, 1.1),
            (4.0, -0.9, -0.6, 2.0),
            (2.2, 1.2, 0.1, 5.5),
            (6.0, -0.1, 1.4, 0.0),
            (1.0, 0.4, -1.3, 3.3),
        ] {
            let m = eq_to_horizon(lat, lst);
            let (az, alt) = vec_horiz(mul(&m, radec_vec(ra, dec)));
            let (az2, alt2) = classical(ra, dec, lat, lst);
            assert!((alt - alt2).abs() < 1e-12, "alt {alt} {alt2}");
            assert!(wrap_pi(az - az2).abs() < 1e-9, "az {az} {az2}");
            // A rotation: orthonormal, determinant +1.
            let t = matmul(&m, &transpose(&m));
            for (i, row) in t.iter().enumerate() {
                for (j, v) in row.iter().enumerate() {
                    assert!((v - if i == j { 1.0 } else { 0.0 }).abs() < 1e-12);
                }
            }
        }
    }

    #[test]
    fn the_pole_stands_at_the_latitude_due_north() {
        for lat in [-60.0, 0.0, 40.0, 89.0] {
            let m = eq_to_horizon(lat * DEG, 1.234);
            let (az, alt) = vec_horiz(mul(&m, [0.0, 1.0, 0.0]));
            assert!((alt.to_degrees() - lat).abs() < 1e-9);
            if lat > 0.0 {
                assert!(az.abs() < 1e-9 || (az - TAU).abs() < 1e-9, "due north");
            }
        }
    }

    #[test]
    fn meeus_example_13b_venus_from_washington() {
        // Meeus, Astronomical Algorithms, example 13.b: 1987-04-10 19:21:00
        // UT, Venus at RA 347.3193, Dec -6.7198, observer 38 55' 17" N,
        // 77 03' 56" W. Meeus gives h = 15.1249 and A = 68.0337 measured
        // from the south, so 248.0337 from the north. He uses apparent
        // sidereal time; ours is mean (0.001 degrees apart that day).
        let t = 545_080_860.0; // 1987-04-10 19:21:00 UTC
        let obs = Observer {
            lat: (38.0 + 55.0 / 60.0 + 17.0 / 3600.0) * DEG,
            lon: -(77.0 + 3.0 / 60.0 + 56.0 / 3600.0) * DEG,
            unix_secs: t,
        };
        let h = mul(
            &obs.eq_to_horizon(),
            radec_vec(347.3193 * DEG, -6.7198 * DEG),
        );
        let (az, alt) = vec_horiz(h);
        assert!(
            (alt.to_degrees() - 15.1249).abs() < 0.005,
            "{}",
            alt.to_degrees()
        );
        assert!(
            (az.to_degrees() - 248.0337).abs() < 0.005,
            "{}",
            az.to_degrees()
        );
    }

    #[test]
    fn the_basis_looks_where_asked_with_the_zenith_up() {
        for &(az, alt) in &[(0.0, 0.0), (1.0, 0.5), (4.0, -1.0), (2.0, ALT_LIMIT)] {
            let cam = SkyCamera::new(az, alt, 1.0);
            let b = cam.basis();
            let back = [b[0][2], b[1][2], b[2][2]];
            let forward = horiz_vec(az, alt);
            assert!(close(back.map(|v| -v), forward, 1e-12));
            let up = [b[0][1], b[1][1], b[2][1]];
            assert!(up[1] >= -1e-12, "up never points below the horizon plane");
            let right = [b[0][0], b[1][0], b[2][0]];
            assert!(right[1].abs() < 1e-12, "no roll");
            let t = matmul(&b, &transpose(&b));
            assert!((t[0][0] - 1.0).abs() < 1e-12 && t[0][1].abs() < 1e-12);
        }
        // Facing north, east is on the right; facing south, on the left.
        let north = SkyCamera::new(0.0, 0.0, 1.0);
        let (x, _) = north.project(horiz_vec(10.0 * DEG, 0.0)).unwrap();
        assert!(x > north.width / 2.0);
        let south = SkyCamera::new(PI, 0.0, 1.0);
        let (x, _) = south.project(horiz_vec(170.0 * DEG, 0.0)).unwrap();
        assert!(x < south.width / 2.0);
    }

    #[test]
    fn project_and_pick_round_trip() {
        let mut cam = SkyCamera::new(2.0, 0.6, 90.0 * DEG);
        cam.width = 1000.0;
        cam.height = 640.0;
        let center = cam.pick(500.0, 320.0).unwrap();
        assert!(close(center, horiz_vec(2.0, 0.6), 1e-12));
        for &(x, y) in &[(0.0, 0.0), (999.0, 639.0), (250.0, 600.0), (500.0, 320.0)] {
            let h = cam.pick(x, y).unwrap();
            let (px, py) = cam.project(h).unwrap();
            assert!(
                (px - x).abs() < 1e-6 && (py - y).abs() < 1e-6,
                "{x},{y} -> {px},{py}"
            );
        }
        // The field of view spans the shorter axis.
        let top = cam.pick(500.0, 0.0).unwrap();
        let bottom = cam.pick(500.0, 640.0).unwrap();
        assert!((angle(top, bottom) - 90.0 * DEG).abs() < 1e-9);
        // Straight behind has no screen position; NaN neither.
        assert_eq!(cam.project(horiz_vec(2.0 + PI, -0.6)), None);
        assert_eq!(cam.project([f64::NAN, 0.0, 0.0]), None);
        assert_eq!(cam.pick(f64::NAN, 1.0), None);
        // Up the screen is up the sky.
        let (_, y) = cam.project(horiz_vec(2.0, 0.7)).unwrap();
        assert!(y < 320.0);
    }

    #[test]
    fn dragging_moves_the_sky_with_the_pointer() {
        let mut cam = SkyCamera::new(PI, 0.5, 60.0 * DEG);
        let target = cam.pick(500.0, 200.0).unwrap();
        cam.drag(500.0, 200.0, 420.0, 260.0);
        let (x, y) = cam.project(target).unwrap();
        assert!(
            (x - 420.0).abs() < 2.0 && (y - 260.0).abs() < 2.0,
            "{x},{y}"
        );
        // Dragging down far looks up, but stops short of the zenith.
        for _ in 0..50 {
            cam.drag(400.0, 100.0, 400.0, 500.0);
        }
        assert!((cam.alt - ALT_LIMIT).abs() < 1e-12);
        cam.drag(f64::NAN, 0.0, 1.0, 1.0);
        assert!(cam.az.is_finite() && cam.alt.is_finite());
    }

    #[test]
    fn zoom_keeps_the_anchor_and_respects_the_limits() {
        let mut cam = SkyCamera::new(1.0, 0.3, 90.0 * DEG);
        let target = cam.pick(650.0, 150.0).unwrap();
        cam.zoom(0.5, Some((650.0, 150.0)));
        assert!((cam.fov - 45.0 * DEG).abs() < 1e-12);
        let (x, y) = cam.project(target).unwrap();
        assert!(
            (x - 650.0).abs() < 0.1 && (y - 150.0).abs() < 0.1,
            "{x},{y}"
        );
        cam.zoom(1e-6, None);
        assert_eq!(cam.fov, FOV_MIN);
        cam.zoom(1e6, None);
        assert_eq!(cam.fov, FOV_MAX);
        cam.zoom(f64::NAN, None);
        cam.zoom(-1.0, None);
        assert_eq!(cam.fov, FOV_MAX);
    }

    #[test]
    fn gestures_drag_and_pinch() {
        let mut cam = SkyCamera::new(PI, 0.4, 80.0 * DEG);
        let mut g = Gestures::default();
        assert!(!g.moved(&mut cam, 1, 10.0, 10.0), "no pointer down");
        g.down(1, 400.0, 300.0);
        let az = cam.az;
        assert!(g.moved(&mut cam, 1, 450.0, 300.0));
        assert!(cam.az < az, "dragging right looks left");
        g.down(2, 500.0, 300.0);
        let fov = cam.fov;
        // Spread the fingers to twice the distance: zoom in 2x.
        assert!(g.moved(&mut cam, 2, 550.0, 300.0));
        assert!((cam.fov - fov / 2.0).abs() < 1e-9, "{} {}", cam.fov, fov);
        g.up(1);
        g.up(2);
        assert!(!g.active());
        g.down(3, f64::NAN, 0.0);
        assert!(!g.active());
    }

    #[test]
    fn nudge_and_wheel() {
        let mut cam = SkyCamera::new(PI, 0.0, 60.0 * DEG);
        cam.nudge(1.0, 0.0);
        assert!((cam.az - (PI + NUDGE * 60.0 * DEG)).abs() < 1e-12);
        cam.nudge(0.0, -1.0);
        assert!((cam.alt + NUDGE * 60.0 * DEG).abs() < 1e-12);
        assert!(
            wheel_factor(100.0, 0, false) > 1.0,
            "scrolling down zooms out"
        );
        assert!(wheel_factor(-100.0, 0, false) < 1.0);
        assert_eq!(wheel_factor(f64::NAN, 0, false), 1.0);
        assert!((wheel_factor(1e9, 2, false) - 1f64.exp()).abs() < 1e-12);
    }

    #[test]
    fn hash_round_trip() {
        let cam = SkyCamera::new(123.4 * DEG, -12.3 * DEG, 5.25 * DEG);
        let h = format!("#10.0,20.0,800&layer=imagery{}", format_hash(&cam));
        assert_eq!(
            h,
            "#10.0,20.0,800&layer=imagery&view=sky&look=123.4,-12.3,5.25"
        );
        assert_eq!(crate::view::View::from_hash(&h), crate::view::View::Sky);
        let back = parse_hash_look(&h).unwrap();
        assert!((back.az - cam.az).abs() < 1e-9 && (back.fov - cam.fov).abs() < 1e-9);
        assert_eq!(parse_hash_look("#1,2,3&look=1,2"), None);
        assert_eq!(parse_hash_look("#1,2,3&look=1,NaN,3"), None);
        assert_eq!(parse_hash_look("#1,2,3&look=1,x,3"), None);
        // Out of range values are clamped and wrapped.
        let c = parse_hash_look("#1,2,3&look=-90,95,500").unwrap();
        assert!((c.az - 270.0 * DEG).abs() < 1e-9);
        assert_eq!((c.alt, c.fov), (ALT_LIMIT, FOV_MAX));
    }

    #[test]
    fn hud_text() {
        assert_eq!(format_ra(0.0), "0h 00m");
        assert_eq!(format_ra(83.82 * DEG), "5h 35m");
        assert_eq!(format_ra(TAU - 1e-9), "0h 00m");
        assert_eq!(
            format_radec(83.82 * DEG, -5.39 * DEG),
            "RA 5h 35m, Dec -5.4°"
        );
        assert_eq!(compass_point(0.0), "N");
        assert_eq!(compass_point(100.0 * DEG), "E");
        assert_eq!(compass_point(350.0 * DEG), "N");
        assert_eq!(compass_point(225.0 * DEG), "SW");
        assert_eq!(
            format_look(&SkyCamera::new(PI, 30.0 * DEG, 1.0)),
            "Az 180° S, Alt 30°"
        );
    }

    #[test]
    fn horizon_labels_are_the_compass_points_in_view() {
        let cam = SkyCamera::new(PI, 0.2, 90.0 * DEG);
        let names: Vec<&str> = horizon_labels(&cam).iter().map(|l| l.2).collect();
        assert_eq!(names, vec!["SE", "S", "SW"]);
        let (x, y, _) = horizon_labels(&cam)[1];
        assert!(
            (x - cam.width / 2.0).abs() < 1e-6 && y > cam.height / 2.0,
            "south, below center"
        );
        let wide = SkyCamera::new(0.0, 1.2, FOV_MAX);
        assert!(horizon_labels(&wide).len() >= 5);
    }

    #[test]
    fn grid_steps_divide_the_circle() {
        for fov in [0.5, 3.0, 20.0, 60.0, 100.0, 150.0] {
            let s = grid_step(fov * DEG).to_degrees();
            assert!((90.0 / s).fract().abs() < 1e-9 && (360.0 / s).fract().abs() < 1e-9);
            assert!(fov / s <= 6.0 + 1e-9 || s == 30.0);
        }
        assert_eq!(grid_step(100.0 * DEG).to_degrees().round(), 30.0);
        assert_eq!(grid_step(60.0 * DEG).to_degrees().round(), 10.0);
    }

    #[test]
    fn to_gl_is_column_major() {
        let m: Mat3 = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]];
        assert_eq!(to_gl(&m), [1.0, 4.0, 7.0, 2.0, 5.0, 8.0, 3.0, 6.0, 9.0]);
    }
}
