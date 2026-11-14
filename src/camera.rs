//! Orbit camera around a unit-radius globe.
//!
//! World space: +Y is the north pole, longitude 0 lies on +Z and 90°E on +X.
//! Camera space: the camera sits at `(0, 0, dist)` looking down -Z, with +Y
//! pointing north on screen ("north up").
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
/// Keep the view center away from the poles so north-up stays well defined.
const MAX_LAT: f64 = 85.0 * PI / 180.0;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Latitude of the point under the screen center, radians.
    pub lat: f64,
    /// Longitude of the point under the screen center, radians.
    pub lon: f64,
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
            alt,
            width: 1.0,
            height: 1.0,
        }
    }

    pub fn dist(&self) -> f64 {
        1.0 + self.alt
    }

    /// Half of the shorter viewport side; the field of view spans this.
    fn half_min(&self) -> f64 {
        0.5 * self.width.min(self.height).max(1.0)
    }

    /// Camera axes expressed in world space: east, north, and out of the screen.
    fn basis(&self) -> [V3; 3] {
        let (sl, cl) = self.lat.sin_cos();
        let (so, co) = self.lon.sin_cos();
        [
            [co, 0.0, -so],
            [-sl * so, cl, -sl * co],
            [cl * so, sl, cl * co],
        ]
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
    /// grabbing the surface at every zoom level.
    pub fn pan_pixels(&mut self, dx: f64, dy: f64) {
        let rate = self.alt * TAN_HALF_FOV / self.half_min();
        self.lat = (self.lat + dy * rate).clamp(-MAX_LAT, MAX_LAT);
        self.lon = wrap_angle(self.lon - dx * rate / self.lat.cos().max(0.2));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        let mut c = Camera::new(30.0, -40.0, 1.5);
        c.width = 1200.0;
        c.height = 800.0;
        c
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
}
