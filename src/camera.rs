//! Orbit camera around a unit-radius globe.
//!
//! World space: +Y is the north pole, longitude 0 lies on +Z and 90°E on +X.
//! Camera space: the camera sits at `(0, 0, dist)` looking down -Z, with +Y
//! pointing up on screen.
//!
//! The orientation is stored as the view center (`lat`, `lon`) plus a
//! `bearing`, the compass direction at the top of the screen. Bearing 0 is
//! "north up". Every orientation is representable, including exactly on a
//! pole, where `lon` only picks which meridian bearing 0 refers to. Panning
//! is a trackball rotation that carries the grabbed surface point with the
//! pointer. In north-up mode the roll is then snapped back to bearing 0, or
//! to 180° after the view has tilted over a pole (north then points down).
//! In free mode the roll is left alone.
//!
//! The same conventions are used by `shaders/globe.frag`, so picking and
//! projection here match what is drawn pixel-for-pixel.

use std::f64::consts::PI;

/// Closest the camera may get to the surface, in globe radii (~380 km).
pub const MIN_ALT: f64 = 0.06;
/// Farthest the camera may get from the surface, in globe radii.
pub const MAX_ALT: f64 = 6.0;
/// tan(fov / 2) for a 40° field of view across the shorter screen axis.
pub const TAN_HALF_FOV: f64 = 0.363_970_234_266_202_4;
pub const EARTH_RADIUS_KM: f64 = 6371.0;
/// Closer to a pole than this (radians), the meridian direction is
/// undefined, so the north-up snap leaves the bearing alone.
const POLE_EPS: f64 = 1e-7;
/// How far the atmosphere halo reaches beyond the surface, in globe radii.
/// `globe.frag` adds `0.55 * exp(-22 * (dmin - 1))` of blue to the space
/// color; at 0.42 radii that is below 6e-5 (a 65 000th of full scale, far under
/// half of an 8-bit step), so nothing past it is shaded.
pub const HALO_EXTENT: f64 = 0.42;
/// Extra margin around the globe quad, in device pixels, so edge pixels
/// whose centers fall just inside the true silhouette are never clipped.
const QUAD_PAD_PX: f64 = 2.0;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Latitude of the point under the screen center, radians.
    pub lat: f64,
    /// Longitude of the point under the screen center, radians.
    pub lon: f64,
    /// Compass direction at the top of the screen, radians clockwise from
    /// north, wrapped into [-π, π]. 0 is north up.
    pub bearing: f64,
    /// North-up lock: panning snaps the bearing to 0 (or π, tilted over a
    /// pole). Off means free, trackball-style rotation.
    pub north_up: bool,
    /// Height above the surface, in globe radii.
    pub alt: f64,
    /// Viewport size in CSS pixels.
    pub width: f64,
    pub height: f64,
}

type V3 = [f64; 3];

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: V3) -> V3 {
    let l = dot(v, v).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

/// Unit vector for a latitude/longitude pair (radians).
pub fn spherical(lat: f64, lon: f64) -> V3 {
    let (sl, cl) = lat.sin_cos();
    let (so, co) = lon.sin_cos();
    [cl * so, sl, cl * co]
}

/// Wrap an angle into [-π, π).
pub fn wrap_angle(a: f64) -> f64 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}

impl Camera {
    pub fn new(lat_deg: f64, lon_deg: f64, alt: f64) -> Self {
        Self {
            lat: lat_deg.to_radians(),
            lon: lon_deg.to_radians(),
            bearing: 0.0,
            north_up: true,
            alt,
            width: 1.0,
            height: 1.0,
        }
    }

    /// A camera with a bearing in degrees. A nonzero bearing means free mode.
    pub fn with_bearing(mut self, bearing_deg: f64) -> Self {
        self.bearing = wrap_angle(bearing_deg.to_radians());
        self.north_up = self.bearing == 0.0;
        self
    }

    /// Bearing in degrees in [0, 360), as shown by the compass.
    pub fn bearing_deg(&self) -> f64 {
        wrap_angle(self.bearing).to_degrees().rem_euclid(360.0)
    }

    pub fn dist(&self) -> f64 {
        1.0 + self.alt
    }

    /// Half of the shorter viewport side; the field of view spans this.
    fn half_min(&self) -> f64 {
        0.5 * self.width.min(self.height).max(1.0)
    }

    /// Ground distance in km covered by one CSS pixel at the view center.
    /// Not finite for a degenerate camera (NaN/∞ altitude).
    pub fn km_per_pixel(&self) -> f64 {
        self.alt * TAN_HALF_FOV / self.half_min() * EARTH_RADIUS_KM
    }

    /// Camera axes expressed in world space: right, up, and out of the screen.
    fn basis(&self) -> [V3; 3] {
        let (sl, cl) = self.lat.sin_cos();
        let (so, co) = self.lon.sin_cos();
        let east = [co, 0.0, -so];
        let north = [-sl * so, cl, -sl * co];
        let out = [cl * so, sl, cl * co];
        let (sb, cb) = self.bearing.sin_cos();
        let mut right = [0.0; 3];
        let mut up = [0.0; 3];
        for i in 0..3 {
            right[i] = cb * east[i] - sb * north[i];
            up[i] = sb * east[i] + cb * north[i];
        }
        [right, up, out]
    }

    /// Set center and bearing from an orthonormal basis (right, up, out).
    /// Within `POLE_EPS` of a pole the old longitude is kept, so the result
    /// is stable there and the bearing carries the whole orientation.
    fn set_basis(&mut self, b: [V3; 3]) {
        let out = b[2];
        let h = out[0].hypot(out[2]);
        self.lat = out[1].atan2(h);
        if h > 1e-9 {
            self.lon = out[0].atan2(out[2]);
        }
        let (sl, cl) = self.lat.sin_cos();
        let (so, co) = self.lon.sin_cos();
        let east = [co, 0.0, -so];
        let north = [-sl * so, cl, -sl * co];
        self.bearing = dot(b[1], east).atan2(dot(b[1], north));
    }

    /// Spin the view about the screen center. Positive is clockwise on
    /// screen, like a twist of two fingers.
    pub fn twist(&mut self, angle: f64) {
        self.bearing = wrap_angle(self.bearing - angle);
    }

    /// Snap the roll to north up (bearing 0), or to 180° when the view has
    /// tilted over a pole. Leaves the bearing alone on a pole itself.
    pub fn snap_north_up(&mut self) {
        if self.lat.cos() > POLE_EPS {
            self.bearing = if self.bearing.cos() >= 0.0 { 0.0 } else { PI };
        }
    }

    /// Rotate the globe about a camera-space axis `(ax, ay, 0)`, by its length.
    fn trackball(&mut self, ax: f64, ay: f64) {
        let theta = ax.hypot(ay);
        if theta == 0.0 || !theta.is_finite() {
            return;
        }
        let k = [ax / theta, ay / theta, 0.0];
        let (s, c) = theta.sin_cos();
        let b = self.basis();
        let mut nb = [[0.0; 3]; 3];
        for (j, col) in nb.iter_mut().enumerate() {
            // m = Rot(-axis, theta) * e_j, in camera coordinates.
            let mut e = [0.0; 3];
            e[j] = 1.0;
            let kxe = [
                k[1] * e[2] - k[2] * e[1],
                k[2] * e[0] - k[0] * e[2],
                k[0] * e[1] - k[1] * e[0],
            ];
            let kde = dot(k, e);
            let m: [f64; 3] =
                std::array::from_fn(|i| e[i] * c - kxe[i] * s + k[i] * kde * (1.0 - c));
            for (i, v) in col.iter_mut().enumerate() {
                *v = m[0] * b[0][i] + m[1] * b[1][i] + m[2] * b[2][i];
            }
        }
        self.set_basis(nb);
    }

    /// Camera-to-world rotation as a column-major 3×3 matrix for GLSL.
    pub fn rotation(&self) -> [f32; 9] {
        let b = self.basis();
        let mut m = [0.0f32; 9];
        for (col, axis) in b.iter().enumerate() {
            for row in 0..3 {
                m[col * 3 + row] = axis[row] as f32;
            }
        }
        m
    }

    /// Camera-space ray direction through a screen point (CSS pixels).
    fn ray(&self, sx: f64, sy: f64) -> V3 {
        let s = self.half_min();
        let x = (sx - self.width * 0.5) / s * TAN_HALF_FOV;
        let y = -(sy - self.height * 0.5) / s * TAN_HALF_FOV;
        normalize([x, y, -1.0])
    }

    /// Latitude/longitude (radians) of the globe surface under a screen point.
    pub fn pick(&self, sx: f64, sy: f64) -> Option<(f64, f64)> {
        let rd = self.ray(sx, sy);
        let d = self.dist();
        let b = d * rd[2];
        let h = b * b - (d * d - 1.0);
        if h < 0.0 {
            return None;
        }
        let t = -b - h.sqrt();
        let n = [t * rd[0], t * rd[1], d + t * rd[2]];
        let basis = self.basis();
        let w = [
            basis[0][0] * n[0] + basis[1][0] * n[1] + basis[2][0] * n[2],
            basis[0][1] * n[0] + basis[1][1] * n[1] + basis[2][1] * n[2],
            basis[0][2] * n[0] + basis[1][2] * n[1] + basis[2][2] * n[2],
        ];
        Some((w[1].clamp(-1.0, 1.0).asin(), w[0].atan2(w[2])))
    }

    /// Screen position (CSS pixels) of a surface point, if it faces the camera.
    pub fn project(&self, lat: f64, lon: f64) -> Option<(f64, f64)> {
        let w = spherical(lat, lon);
        let b = self.basis();
        let n = [dot(b[0], w), dot(b[1], w), dot(b[2], w)];
        let d = self.dist();
        if n[2] <= 1.0 / d {
            return None; // over the horizon
        }
        let depth = d - n[2];
        let s = self.half_min();
        let x = n[0] / depth / TAN_HALF_FOV;
        let y = n[1] / depth / TAN_HALF_FOV;
        Some((self.width * 0.5 + x * s, self.height * 0.5 - y * s))
    }

    /// Move the globe so its surface follows the pointer by (dx, dy) pixels.
    ///
    /// The rate is exact at the screen center, which makes dragging feel like
    /// grabbing the surface at every zoom level. This is a trackball rotation,
    /// so it is well defined everywhere, including over the poles.
    pub fn pan_pixels(&mut self, dx: f64, dy: f64) {
        let rate = self.alt * TAN_HALF_FOV / self.half_min();
        self.trackball(dy * rate, dx * rate);
        if self.north_up {
            self.snap_north_up();
        }
    }

    /// Nudge the view so that a surface point sits under a screen position.
    /// Used to zoom toward the cursor and to pan while pinching.
    pub fn keep_point_at(&mut self, lat: f64, lon: f64, sx: f64, sy: f64) {
        for _ in 0..4 {
            match self.project(lat, lon) {
                Some((px, py)) => {
                    let (dx, dy) = (sx - px, sy - py);
                    if dx.abs() < 0.05 && dy.abs() < 0.05 {
                        break;
                    }
                    self.pan_pixels(dx, dy);
                }
                None => break,
            }
        }
    }

    /// Screen rectangle `[x0, y0, x1, y1]` in normalized device coordinates
    /// (y up) that bounds the globe and its atmosphere halo, for a render
    /// target of `w_px × h_px` pixels. The camera looks straight at the
    /// center, so the silhouette is a circle around the screen center:
    /// a ray with closest approach `r` to the center leaves the optical axis
    /// at angle `asin(r / dist)`. If the camera is inside the halo radius the
    /// whole screen is covered.
    pub fn globe_bounds(&self, w_px: f64, h_px: f64) -> [f64; 4] {
        let full = [-1.0, -1.0, 1.0, 1.0];
        let r = 1.0 + HALO_EXTENT;
        let d = self.dist();
        if !(d.is_finite() && w_px >= 1.0 && h_px >= 1.0) || d <= r {
            return full;
        }
        // Radius on screen in units of half the shorter side.
        let rp = r / (d * d - r * r).sqrt() / TAN_HALF_FOV;
        let half_min = 0.5 * w_px.min(h_px);
        let hx = ((rp * half_min + QUAD_PAD_PX) / (0.5 * w_px)).min(1.0);
        let hy = ((rp * half_min + QUAD_PAD_PX) / (0.5 * h_px)).min(1.0);
        [-hx, -hy, hx, hy]
    }

    /// Graticule spacing in degrees appropriate for the current zoom.
    pub fn grid_step_deg(&self) -> f64 {
        match self.alt {
            a if a > 1.5 => 30.0,
            a if a > 0.6 => 15.0,
            a if a > 0.25 => 10.0,
            a if a > 0.1 => 5.0,
            _ => 2.0,
        }
    }
}

/// Factor the shader applies to the longitude (u) texture gradient at
/// latitude `lat` (radians) so the sampled footprint matches surface area:
/// a degree of longitude spans cos(lat) of a degree of latitude. Clamped so
/// the LOD stays finite at the poles. Mirrors `uScale` in `globe.frag`
/// (test-only: the shader is the real implementation).
#[cfg(test)]
pub fn polar_u_scale(lat: f64) -> f64 {
    let c = lat.cos();
    if c.is_nan() {
        1.0
    } else {
        c.max(0.02)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn polar_u_scale_matches_surface_footprint() {
        assert_eq!(polar_u_scale(0.0), 1.0);
        let lat = 60f64.to_radians();
        assert!((polar_u_scale(lat) - 0.5).abs() < 1e-12);
        // East-west ground distance per unit u shrinks by exactly this factor.
        let lat = 85f64.to_radians();
        assert!((polar_u_scale(lat) - lat.cos()).abs() < 1e-12);
        // Poles and bad input stay finite and positive.
        for l in [
            std::f64::consts::FRAC_PI_2,
            -std::f64::consts::FRAC_PI_2,
            f64::NAN,
            f64::INFINITY,
        ] {
            let s = polar_u_scale(l);
            assert!(s.is_finite() && s > 0.0 && s <= 1.0);
        }
    }

    use super::*;

    fn cam() -> Camera {
        let mut c = Camera::new(30.0, -40.0, 1.5);
        c.width = 1200.0;
        c.height = 800.0;
        c
    }

    /// Closest approach of the ray through a screen point to the globe center.
    fn ray_dmin(c: &Camera, sx: f64, sy: f64) -> f64 {
        let rd = c.ray(sx, sy);
        let b = c.dist() * rd[2];
        (c.dist() * c.dist() - b * b).max(0.0).sqrt()
    }

    /// Bounds in CSS pixels for a camera whose buffer equals its viewport.
    fn bounds_px(c: &Camera) -> [f64; 4] {
        let b = c.globe_bounds(c.width, c.height);
        let to = |n: f64, size: f64| (n + 1.0) * 0.5 * size;
        [
            to(b[0], c.width),
            to(-b[3], c.height),
            to(b[2], c.width),
            to(-b[1], c.height),
        ]
    }

    /// Every pixel outside the bounds is shaded identically to empty space
    /// (halo below threshold), and the whole visible globe is inside.
    #[test]
    fn bounds_contain_globe_and_halo_at_every_altitude() {
        for (w, h) in [(1200.0, 800.0), (390.0, 844.0), (900.0, 900.0)] {
            for alt in [MIN_ALT, 0.2, 0.5, 1.0, 1.5, 3.0, MAX_ALT] {
                let mut c = Camera::new(20.0, 30.0, alt);
                c.width = w;
                c.height = h;
                let [x0, y0, x1, y1] = bounds_px(&c);
                let mut x = 0.5;
                while x < w {
                    let mut y = 0.5;
                    while y < h {
                        let inside = x >= x0 && x <= x1 && y >= y0 && y <= y1;
                        if !inside {
                            assert!(
                                ray_dmin(&c, x, y) > 1.0 + HALO_EXTENT,
                                "alt {alt} {w}x{h}: ({x},{y}) outside bounds but within halo"
                            );
                            assert!(c.pick(x, y).is_none());
                        }
                        y += 3.0;
                    }
                    x += 3.0;
                }
            }
        }
    }

    /// Surface points project (via `project`) inside the bounds, including
    /// points right at the limb.
    #[test]
    fn projected_surface_points_lie_inside_bounds() {
        for alt in [MIN_ALT, 0.3, 1.0, 2.5, MAX_ALT] {
            let mut c = Camera::new(-35.0, 100.0, alt);
            c.width = 1000.0;
            c.height = 700.0;
            let [x0, y0, x1, y1] = bounds_px(&c);
            for lat in (-89..=89).step_by(2) {
                for lon in (-180..180).step_by(3) {
                    if let Some((sx, sy)) = c.project(lat as f64, lon as f64) {
                        if !(0.0..=c.width).contains(&sx) || !(0.0..=c.height).contains(&sy) {
                            continue; // off screen anyway
                        }
                        assert!(
                            sx >= x0 && sx <= x1 && sy >= y0 && sy <= y1,
                            "alt {alt}: ({lat},{lon}) -> ({sx},{sy}) outside"
                        );
                    }
                }
            }
        }
    }

    /// The bound is tight: far away it is much smaller than the screen, and
    /// its edge sits where the halo radius leaves the viewport axis.
    #[test]
    fn bounds_are_tight_and_centered() {
        let mut c = Camera::new(0.0, 0.0, MAX_ALT);
        c.width = 1200.0;
        c.height = 800.0;
        let [x0, y0, x1, y1] = bounds_px(&c);
        assert!((x0 + x1 - 1200.0).abs() < 1e-6 && (y0 + y1 - 800.0).abs() < 1e-6);
        assert!(x1 - x0 < 0.4 * 1200.0, "{}", x1 - x0);
        // A pixel just inside the edge (beyond the 2 px pad) is still halo.
        let edge = x1 - QUAD_PAD_PX;
        let dmin = ray_dmin(&c, edge, 400.0);
        assert!((dmin - (1.0 + HALO_EXTENT)).abs() < 5e-3, "{dmin}");
    }

    #[test]
    fn bounds_cover_the_screen_when_inside_the_halo() {
        let mut c = Camera::new(0.0, 0.0, 0.3); // dist 1.3 < 1.42
        c.width = 800.0;
        c.height = 600.0;
        assert_eq!(c.globe_bounds(800.0, 600.0), [-1.0, -1.0, 1.0, 1.0]);
        // Degenerate inputs fall back to the full screen, never a panic.
        c.alt = f64::NAN;
        assert_eq!(c.globe_bounds(800.0, 600.0), [-1.0, -1.0, 1.0, 1.0]);
        c.alt = 2.0;
        assert_eq!(c.globe_bounds(0.0, 0.0), [-1.0, -1.0, 1.0, 1.0]);
    }

    #[test]
    fn bounds_and_silhouette_ignore_bearing_and_pole_position() {
        // The silhouette is a circle about the screen center, so the bounds
        // must not depend on orientation, and surface points must stay inside
        // them at any bearing, poles included.
        for c in many_cams() {
            let mut north = c;
            north.bearing = 0.0;
            north.lat = 0.0;
            assert_eq!(
                c.globe_bounds(c.width, c.height),
                north.globe_bounds(c.width, c.height)
            );
            let [x0, y0, x1, y1] = bounds_px(&c);
            for &(lat, lon) in &[(c.lat, c.lon), (c.lat + 0.9, c.lon + 0.4), (-c.lat, c.lon)] {
                if let Some((px, py)) = c
                    .project(lat.clamp(-PI / 2.0, PI / 2.0), lon)
                    .filter(|&(x, y)| x >= 0.0 && x <= c.width && y >= 0.0 && y <= c.height)
                {
                    assert!(px >= x0 && px <= x1 && py >= y0 && py <= y1, "{c:?}");
                }
            }
        }
    }

    #[test]
    fn bounds_scale_with_buffer_size() {
        // Same camera at half-size buffer gives the same NDC rect (up to pad).
        let mut c = Camera::new(0.0, 0.0, 3.0);
        c.width = 1000.0;
        c.height = 600.0;
        let a = c.globe_bounds(1000.0, 600.0);
        let b = c.globe_bounds(500.0, 300.0);
        for i in 0..4 {
            assert!((a[i] - b[i]).abs() < 0.01, "{a:?} {b:?}");
        }
    }

    #[test]
    fn center_pick_is_view_center() {
        let c = cam();
        let (lat, lon) = c.pick(600.0, 400.0).unwrap();
        assert!((lat - c.lat).abs() < 1e-9);
        assert!((lon - c.lon).abs() < 1e-9);
    }

    #[test]
    fn pick_and_project_round_trip() {
        let c = cam();
        for &(sx, sy) in &[(500.0, 300.0), (700.0, 520.0), (610.0, 140.0)] {
            let (lat, lon) = c.pick(sx, sy).unwrap();
            let (px, py) = c.project(lat, lon).unwrap();
            assert!((px - sx).abs() < 1e-6 && (py - sy).abs() < 1e-6);
        }
    }

    #[test]
    fn keep_point_at_converges() {
        let mut c = cam();
        let (lat, lon) = c.pick(800.0, 300.0).unwrap();
        c.alt *= 0.5;
        c.keep_point_at(lat, lon, 800.0, 300.0);
        let (px, py) = c.project(lat, lon).unwrap();
        assert!((px - 800.0).abs() < 1.0 && (py - 300.0).abs() < 1.0);
    }

    #[test]
    fn back_side_is_hidden() {
        let c = cam();
        assert!(c.project(-c.lat, c.lon + PI).is_none());
    }

    #[test]
    fn wrap_angle_stays_in_range() {
        for a in [-10.0, -PI, -1.0, 0.0, 1.0, PI, 3.5, 10.0, 100.0] {
            let w = wrap_angle(a);
            assert!((-PI..PI).contains(&w), "{a} -> {w}");
            // Same direction on the circle.
            assert!((w.sin() - a.sin()).abs() < 1e-9 && (w.cos() - a.cos()).abs() < 1e-9);
        }
    }

    #[test]
    fn spherical_is_unit_length_with_documented_axes() {
        let n = spherical(PI / 2.0, 0.0);
        assert!((n[1] - 1.0).abs() < 1e-12, "north pole is +Y");
        let z = spherical(0.0, 0.0);
        assert!((z[2] - 1.0).abs() < 1e-12, "lon 0 is +Z");
        let x = spherical(0.0, PI / 2.0);
        assert!((x[0] - 1.0).abs() < 1e-12, "90°E is +X");
        for &(lat, lon) in &[(0.3, -2.0), (-1.2, 0.7), (1.5, 3.0)] {
            assert!((dot(spherical(lat, lon), spherical(lat, lon)) - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn rotation_is_orthonormal() {
        for &(lat, lon) in &[(0.0, 0.0), (60.0, -120.0), (-85.0, 179.0)] {
            let r = Camera::new(lat, lon, 1.0).rotation();
            let col = |i: usize| [r[3 * i] as f64, r[3 * i + 1] as f64, r[3 * i + 2] as f64];
            for i in 0..3 {
                for j in 0..3 {
                    let expected = if i == j { 1.0 } else { 0.0 };
                    assert!((dot(col(i), col(j)) - expected).abs() < 1e-6);
                }
            }
            // Right-handed: x × y = z.
            let (x, y, z) = (col(0), col(1), col(2));
            let cross = [
                x[1] * y[2] - x[2] * y[1],
                x[2] * y[0] - x[0] * y[2],
                x[0] * y[1] - x[1] * y[0],
            ];
            assert!((dot(cross, z) - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn north_is_up_on_screen() {
        let c = cam();
        let (_, center_y) = c.project(c.lat, c.lon).unwrap();
        let (_, north_y) = c.project(c.lat + 0.05, c.lon).unwrap();
        assert!(north_y < center_y);
        let (center_x, _) = c.project(c.lat, c.lon).unwrap();
        let (east_x, _) = c.project(c.lat, c.lon + 0.05).unwrap();
        assert!(east_x > center_x);
    }

    #[test]
    fn pick_outside_globe_is_none() {
        let c = cam();
        assert!(c.pick(0.0, 0.0).is_none());
        assert!(c.pick(1199.0, 799.0).is_none());
    }

    #[test]
    fn drag_moves_surface_with_pointer() {
        // Dragging right/down by a few pixels should carry the point under the
        // cursor along with it.
        let mut c = cam();
        let (sx, sy) = (600.0, 400.0);
        let (lat, lon) = c.pick(sx, sy).unwrap();
        c.pan_pixels(5.0, 3.0);
        let (px, py) = c.project(lat, lon).unwrap();
        assert!((px - (sx + 5.0)).abs() < 0.05, "x: {px}");
        assert!((py - (sy + 3.0)).abs() < 0.05, "y: {py}");
    }

    #[test]
    fn pan_stays_finite_and_crosses_the_poles() {
        let mut c = cam();
        let mut max_lat: f64 = 0.0;
        for _ in 0..1000 {
            c.pan_pixels(37.0, 500.0);
            assert!(c.lat.is_finite() && c.lon.is_finite() && c.bearing.is_finite());
            assert!(c.lat.abs() <= PI / 2.0 + 1e-12);
            assert!((-PI..=PI).contains(&c.lon));
            max_lat = max_lat.max(c.lat.abs());
        }
        assert!(max_lat > 85f64.to_radians(), "reaches past the old clamp");
    }

    /// A deterministic spread of cameras: poles, seam, odd bearings.
    fn many_cams() -> Vec<Camera> {
        let mut out = Vec::new();
        for &lat in &[90.0, -90.0, 89.999, -89.5, 85.0, 45.0, 0.0, -30.0] {
            for &lon in &[0.0, 90.0, 179.999, -180.0, -135.0, 20.0] {
                for &b in &[0.0, 17.0, 90.0, 180.0, -100.0, 359.0] {
                    for &alt in &[MIN_ALT, 0.5, 2.0] {
                        let mut c = Camera::new(lat, lon, alt).with_bearing(b);
                        c.width = 900.0;
                        c.height = 600.0;
                        out.push(c);
                    }
                }
            }
        }
        out
    }

    #[test]
    fn pick_and_project_round_trip_everywhere() {
        for c in many_cams() {
            for &(sx, sy) in &[
                (450.0, 300.0),
                (300.0, 200.0),
                (560.0, 380.0),
                (451.0, 90.0),
            ] {
                let Some((lat, lon)) = c.pick(sx, sy) else {
                    continue;
                };
                let (px, py) = c.project(lat, lon).expect("picked point faces the camera");
                assert!(
                    (px - sx).abs() < 1e-4 && (py - sy).abs() < 1e-4,
                    "{c:?}: ({sx},{sy}) -> ({px},{py})"
                );
            }
            let (lat, lon) = c.pick(450.0, 300.0).unwrap();
            let want = spherical(c.lat, c.lon);
            let got = spherical(lat, lon);
            assert!((dot(want, got) - 1.0).abs() < 1e-12, "{c:?}");
        }
    }

    #[test]
    fn pole_is_centered_exactly() {
        for (lat, sign) in [(90.0, 1.0), (-90.0, -1.0)] {
            let mut c = Camera::new(lat, 33.0, 1.0);
            c.width = 800.0;
            c.height = 800.0;
            let (px, py) = c.project(sign * PI / 2.0, 0.0).unwrap();
            assert!((px - 400.0).abs() < 1e-9 && (py - 400.0).abs() < 1e-9);
            let (plat, _) = c.pick(400.0, 400.0).unwrap();
            assert!((plat - sign * PI / 2.0).abs() < 1e-9);
        }
    }

    #[test]
    fn bearing_rotates_the_view_about_the_center() {
        // At bearing 90 east is up the screen and north is to the left.
        let mut c = Camera::new(10.0, 20.0, 1.5).with_bearing(90.0);
        c.width = 1000.0;
        c.height = 1000.0;
        let (cx, cy) = c.project(c.lat, c.lon).unwrap();
        let (ex, ey) = c.project(c.lat, c.lon + 0.05).unwrap();
        assert!(ey < cy && (ex - cx).abs() < 1.0, "east is up at bearing 90");
        let (nx, ny) = c.project(c.lat + 0.05, c.lon).unwrap();
        assert!(
            nx < cx && (ny - cy).abs() < 1.0,
            "north is left at bearing 90"
        );
        assert!((c.bearing_deg() - 90.0).abs() < 1e-9);
        assert!(!c.north_up, "a bearing means free mode");
    }

    #[test]
    fn rotation_matches_the_shader_model_at_any_bearing() {
        // globe.frag and line.vert only see `rotation()`: ray through a pixel,
        // hit the sphere, rotate into world space. Replicate that in f32 and
        // check it agrees with `pick` for every orientation.
        for c in many_cams().into_iter().step_by(7) {
            let r = c.rotation();
            let col = |i: usize| [r[3 * i] as f64, r[3 * i + 1] as f64, r[3 * i + 2] as f64];
            let (sx, sy) = (c.width * 0.5 + 40.0, c.height * 0.5 - 25.0);
            let Some((lat, lon)) = c.pick(sx, sy) else {
                continue;
            };
            let rd = c.ray(sx, sy);
            let d = c.dist();
            let b = d * rd[2];
            let t = -b - (b * b - (d * d - 1.0)).sqrt();
            let n = [t * rd[0], t * rd[1], d + t * rd[2]];
            // u_rot * n
            let w: V3 =
                std::array::from_fn(|i| col(0)[i] * n[0] + col(1)[i] * n[1] + col(2)[i] * n[2]);
            let want = spherical(lat, lon);
            assert!(dot(w, want) > 1.0 - 1e-6, "{c:?}");
            // line.vert: transpose(u_rot) * p, then the same projection.
            let back = [dot(col(0), want), dot(col(1), want), dot(col(2), want)];
            let depth = d - back[2];
            let px = c.width * 0.5 + back[0] / depth / TAN_HALF_FOV * c.half_min();
            let py = c.height * 0.5 - back[1] / depth / TAN_HALF_FOV * c.half_min();
            assert!((px - sx).abs() < 0.05 && (py - sy).abs() < 0.05, "{c:?}");
        }
    }

    #[test]
    fn rotation_is_orthonormal_at_poles_and_bearings() {
        for c in many_cams() {
            let r = c.rotation();
            let col = |i: usize| [r[3 * i] as f64, r[3 * i + 1] as f64, r[3 * i + 2] as f64];
            for i in 0..3 {
                for j in 0..3 {
                    let e = if i == j { 1.0 } else { 0.0 };
                    assert!((dot(col(i), col(j)) - e).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn drag_follows_the_cursor_at_the_poles() {
        // Free mode follows the cursor in every direction, on the pole too.
        // North-up cannot follow a sideways drag of the pole itself (the
        // globe can only spin about its axis there), so it is checked with
        // vertical drags, and with sideways drags a little way off the pole.
        for north_up in [true, false] {
            for &(lat, b) in &[(90.0, 0.0), (-90.0, 0.0), (90.0, 40.0), (89.99, 0.0)] {
                if north_up && b != 0.0 {
                    continue;
                }
                for alt in [MIN_ALT, 0.4, 2.0] {
                    let mut c = Camera::new(lat, 25.0, alt).with_bearing(b);
                    c.north_up = north_up;
                    c.width = 1000.0;
                    c.height = 700.0;
                    let (sx, sy) = (500.0, 350.0);
                    let (glat, glon) = c.pick(sx, sy).unwrap();
                    let drags: &[(f64, f64)] = if north_up {
                        &[(0.0, -5.0), (0.0, 4.0)]
                    } else {
                        &[(5.0, 0.0), (0.0, -5.0), (-3.0, 4.0)]
                    };
                    for &(dx, dy) in drags {
                        let mut d = c;
                        d.pan_pixels(dx, dy);
                        let (px, py) = d.project(glat, glon).unwrap();
                        assert!(
                            (px - (sx + dx)).abs() < 0.05 && (py - (sy + dy)).abs() < 0.05,
                            "lat {lat} b {b} alt {alt} nu {north_up}: ({px},{py})"
                        );
                    }
                }
            }
        }
        let mut c = Camera::new(85.0, 25.0, MIN_ALT);
        c.width = 1000.0;
        c.height = 700.0;
        let (glat, glon) = c.pick(500.0, 350.0).unwrap();
        c.pan_pixels(5.0, 0.0);
        let (px, py) = c.project(glat, glon).unwrap();
        assert!((px - 505.0).abs() < 0.1 && (py - 350.0).abs() < 0.1);
    }

    #[test]
    fn drag_follows_the_cursor_off_center_near_the_pole() {
        let mut c = Camera::new(88.0, 10.0, 0.4);
        c.north_up = false;
        c.width = 1000.0;
        c.height = 700.0;
        let (sx, sy) = (620.0, 300.0);
        let (glat, glon) = c.pick(sx, sy).unwrap();
        c.pan_pixels(6.0, 4.0);
        let (px, py) = c.project(glat, glon).unwrap();
        // A trackball is exact at the center; elsewhere the error is small.
        assert!((px - (sx + 6.0)).abs() < 1.5 && (py - (sy + 4.0)).abs() < 1.5);
    }

    #[test]
    fn dragging_straight_across_the_pole_is_continuous() {
        // Keep dragging down with the pointer at the screen center: the
        // surface under it follows each step, the view center never jumps,
        // and north-up tilts through the pole to bearing 180.
        let mut c = Camera::new(89.0, 0.0, 0.5);
        c.width = 800.0;
        c.height = 800.0;
        let mut crossed = false;
        let mut prev_center = spherical(c.lat, c.lon);
        let mut min_lat = f64::MAX;
        for _ in 0..400 {
            let (glat, glon) = c.pick(400.0, 400.0).unwrap();
            c.pan_pixels(0.0, 1.0);
            let (px, py) = c.project(glat, glon).unwrap();
            assert!((py - 401.0).abs() < 1e-6 && (px - 400.0).abs() < 1e-6);
            let now = spherical(c.lat, c.lon);
            let step = dot(prev_center, now).clamp(-1.0, 1.0).acos();
            assert!(step < 0.01, "center jumped by {step}");
            prev_center = now;
            min_lat = min_lat.min((PI / 2.0 - c.lat).abs());
            if c.bearing.abs() > 3.0 {
                crossed = true;
            }
        }
        assert!(c.north_up);
        assert!(crossed, "north-up tilts over the pole to bearing 180");
        assert!(min_lat < 0.01, "the drag went through the pole");
    }

    fn lcg(s: &mut u64) -> u64 {
        *s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *s >> 33
    }

    #[test]
    fn north_up_keeps_bearing_zero_or_flipped() {
        let mut c = cam();
        let mut s = 1u64;
        for _ in 0..2000 {
            let dx = (lcg(&mut s) % 200) as f64 - 100.0;
            let dy = (lcg(&mut s) % 200) as f64 - 100.0;
            c.pan_pixels(dx, dy);
            let on_pole = c.lat.cos() <= POLE_EPS;
            assert!(
                on_pole || c.bearing == 0.0 || c.bearing == PI,
                "bearing {}",
                c.bearing
            );
        }
    }

    #[test]
    fn free_mode_pan_never_nans_and_keeps_orthonormal_basis() {
        let mut c = Camera::new(70.0, 10.0, 0.3).with_bearing(33.0);
        c.width = 900.0;
        c.height = 600.0;
        let mut s = 7u64;
        for _ in 0..5000 {
            let dx = (lcg(&mut s) % 400) as f64 - 200.0;
            let dy = (lcg(&mut s) % 400) as f64 - 200.0;
            c.pan_pixels(dx, dy);
            assert!(c.lat.is_finite() && c.lon.is_finite() && c.bearing.is_finite());
        }
        let r = c.rotation();
        let l = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
        assert!((l - 1.0).abs() < 1e-6);
    }

    #[test]
    fn free_mode_pan_does_not_roll_the_view() {
        // Trackball: a small closed loop of drags rolls the view only by the
        // enclosed area, which is tiny.
        let mut c = Camera::new(30.0, 0.0, 1.0).with_bearing(50.0);
        c.width = 1000.0;
        c.height = 1000.0;
        let before = c.bearing;
        for &(dx, dy) in &[(3.0, 0.0), (0.0, 3.0), (-3.0, 0.0), (0.0, -3.0)] {
            c.pan_pixels(dx, dy);
        }
        assert!((c.bearing - before).abs() < 1e-3);
    }

    #[test]
    fn twist_changes_only_the_bearing() {
        let mut c = cam();
        c.north_up = false;
        let center = c.pick(600.0, 400.0).unwrap();
        c.twist(0.5);
        let after = c.pick(600.0, 400.0).unwrap();
        assert!((center.0 - after.0).abs() < 1e-12 && (center.1 - after.1).abs() < 1e-12);
        assert!(
            (c.bearing + 0.5).abs() < 1e-12,
            "clockwise twist lowers the bearing"
        );
        c.twist(10.0);
        assert!((-PI..=PI).contains(&c.bearing));
    }

    #[test]
    fn set_basis_round_trips_orientation() {
        for c in many_cams().into_iter().step_by(3) {
            let mut d = Camera::new(0.0, 0.0, c.alt);
            d.lon = c.lon;
            d.set_basis(c.basis());
            let (a, b) = (c.basis(), d.basis());
            for i in 0..3 {
                for j in 0..3 {
                    assert!((a[i][j] - b[i][j]).abs() < 1e-9, "{c:?} vs {d:?}");
                }
            }
        }
    }

    #[test]
    fn grid_step_gets_finer_as_you_zoom_in() {
        let mut prev = f64::INFINITY;
        let mut c = cam();
        for alt in [MAX_ALT, 2.0, 1.0, 0.4, 0.2, MIN_ALT] {
            c.alt = alt;
            let step = c.grid_step_deg();
            assert!(step <= prev);
            assert!(180.0 % step == 0.0, "grid step must divide 180°");
            prev = step;
        }
    }

    #[test]
    fn portrait_viewport_uses_shorter_axis() {
        // The fov spans the shorter axis, so a globe that fits a landscape
        // screen's height also fits a portrait screen's width.
        let mut c = Camera::new(0.0, 0.0, 2.4);
        c.width = 400.0;
        c.height = 900.0;
        assert!(c.pick(200.0, 450.0).is_some());
        assert!(
            c.pick(2.0, 450.0).is_none(),
            "globe should not touch the sides"
        );
    }
}
