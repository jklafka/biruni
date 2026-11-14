//! Great-circle measurement: pure, browser-free math and state.
//!
//! Distances use the haversine formula on a sphere of radius
//! `EARTH_RADIUS_KM`, which is within about 0.5% of the WGS84 geodesic.

use crate::camera::{spherical, Camera, EARTH_RADIUS_KM};
use crate::view::{Units, KM_PER_MILE};
use std::f64::consts::PI;

/// Most points a path may have.
pub const MAX_POINTS: usize = 100;
/// Angular spacing of samples along a drawn arc, radians (0.5°).
const SAMPLE_STEP: f64 = 0.5 * PI / 180.0;
/// Hard cap on samples per segment.
const MAX_SAMPLES: usize = 720;

type V3 = [f64; 3];

fn is_valid(p: (f64, f64)) -> bool {
    p.0.is_finite() && p.1.is_finite() && p.0.abs() <= PI / 2.0 + 1e-9
}

/// Central angle in radians between two (lat, lon) points in radians, in
/// `[0, π]`. `None` for non-finite input. Stable for tiny and antipodal pairs.
pub fn central_angle(a: (f64, f64), b: (f64, f64)) -> Option<f64> {
    if !is_valid(a) || !is_valid(b) {
        return None;
    }
    let (dlat, dlon) = (b.0 - a.0, b.1 - a.1);
    let h = (dlat / 2.0).sin().powi(2) + a.0.cos() * b.0.cos() * (dlon / 2.0).sin().powi(2);
    let h = h.clamp(0.0, 1.0);
    Some(2.0 * h.sqrt().atan2((1.0 - h).sqrt()))
}

/// Great-circle distance in km. `None` for non-finite input.
pub fn distance_km(a: (f64, f64), b: (f64, f64)) -> Option<f64> {
    central_angle(a, b).map(|c| c * EARTH_RADIUS_KM)
}

fn to_latlon(v: V3) -> (f64, f64) {
    (v[1].clamp(-1.0, 1.0).asin(), v[0].atan2(v[2]))
}

/// Point a fraction `f` (0 = `a`, 1 = `b`) along the great circle from `a`
/// to `b`. For antipodal points, where every great circle qualifies, the
/// path goes over the north pole (or along the x axis from a pole). `None`
/// for non-finite input or `f`.
pub fn interpolate(a: (f64, f64), b: (f64, f64), f: f64) -> Option<(f64, f64)> {
    if !f.is_finite() {
        return None;
    }
    let delta = central_angle(a, b)?;
    let va = spherical(a.0, a.1);
    let vb = spherical(b.0, b.1);
    if delta < 1e-12 {
        return Some(a);
    }
    let v: V3 = if PI - delta < 1e-9 {
        // Antipodal: rotate `a` toward a perpendicular chosen deterministically.
        let up = if va[1].abs() < 0.999 {
            [0.0, 1.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let mut p = [
            up[1] * va[2] - up[2] * va[1],
            up[2] * va[0] - up[0] * va[2],
            up[0] * va[1] - up[1] * va[0],
        ];
        let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        p = [p[0] / l, p[1] / l, p[2] / l];
        let (s, c) = (f * PI).sin_cos();
        [
            va[0] * c + p[0] * s,
            va[1] * c + p[1] * s,
            va[2] * c + p[2] * s,
        ]
    } else {
        let s = delta.sin();
        let (wa, wb) = (((1.0 - f) * delta).sin() / s, (f * delta).sin() / s);
        [
            wa * va[0] + wb * vb[0],
            wa * va[1] + wb * vb[1],
            wa * va[2] + wb * vb[2],
        ]
    };
    Some(to_latlon(v))
}

/// Points along the great circle from `a` to `b`, endpoints included, spaced
/// at most half a degree apart. Empty for non-finite input.
pub fn sample_arc(a: (f64, f64), b: (f64, f64)) -> Vec<(f64, f64)> {
    let Some(delta) = central_angle(a, b) else {
        return Vec::new();
    };
    let n = ((delta / SAMPLE_STEP).ceil() as usize).clamp(1, MAX_SAMPLES);
    let mut out = Vec::with_capacity(n + 1);
    out.push(a);
    for i in 1..n {
        if let Some(p) = interpolate(a, b, i as f64 / n as f64) {
            out.push(p);
        }
    }
    out.push(b);
    out
}

/// A path of points on the globe being measured.
#[derive(Clone, Debug, Default)]
pub struct Measure {
    pub active: bool,
    points: Vec<(f64, f64)>,
}

impl Measure {
    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }

    /// Add a point (radians). Returns false if it was rejected: non-finite,
    /// or the path is full.
    pub fn add(&mut self, lat: f64, lon: f64) -> bool {
        if !is_valid((lat, lon)) || self.points.len() >= MAX_POINTS {
            return false;
        }
        self.points.push((lat, lon));
        true
    }

    /// Remove the last point. Returns false if there was none.
    pub fn undo(&mut self) -> bool {
        self.points.pop().is_some()
    }

    /// Drop all points and leave measure mode.
    pub fn exit(&mut self) {
        self.points.clear();
        self.active = false;
    }

    /// Length of each segment in km.
    pub fn legs_km(&self) -> Vec<f64> {
        self.points
            .windows(2)
            .map(|w| distance_km(w[0], w[1]).unwrap_or(0.0))
            .collect()
    }

    pub fn total_km(&self) -> f64 {
        // `Iterator::sum` of no floats is -0.0, which printed as "-0.0 km".
        self.legs_km().iter().fold(0.0, |a, b| a + b)
    }
}

/// Distance as text with thousands separators, e.g. `5,570 km` or
/// `3,461 mi`. Below 10 shows one decimal, and zero shows
/// `0 km`; non-finite shows an em dash.
pub fn format_distance(km: f64, units: Units) -> String {
    if !km.is_finite() || km < 0.0 {
        return "—".into();
    }
    let (v, unit) = match units {
        Units::Metric => (km, "km"),
        Units::Imperial => (km / KM_PER_MILE, "mi"),
    };
    if v < 10.0 {
        // Zero (including -0.0 from `0.0 * -1.0` style arithmetic) and
        // values that round to 0.0 print as a plain "0".
        if (v * 10.0).round() == 0.0 {
            return format!("0 {unit}");
        }
        return format!("{v:.1} {unit}");
    }
    let digits = (v.round() as u64).to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    format!("{out} {unit}")
}

/// Screen-space polylines (CSS pixels) for the path. Each is a run of
/// consecutive samples that face the camera, so segments hidden behind the
/// globe are not drawn.
pub fn screen_path(cam: &Camera, points: &[(f64, f64)]) -> Vec<Vec<(f64, f64)>> {
    let mut runs: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut cur: Vec<(f64, f64)> = Vec::new();
    let mut joined = false; // `cur` ends at the previous segment's last sample
    for w in points.windows(2) {
        for (i, p) in sample_arc(w[0], w[1]).into_iter().enumerate() {
            if i == 0 && joined {
                continue; // repeats the previous segment's last sample
            }
            match cam.project(p.0, p.1) {
                Some(s) => cur.push(s),
                None => {
                    if cur.len() > 1 {
                        runs.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                }
            }
        }
        joined = !cur.is_empty();
    }
    if cur.len() > 1 {
        runs.push(cur);
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deg(lat: f64, lon: f64) -> (f64, f64) {
        (lat.to_radians(), lon.to_radians())
    }

    fn within(actual: f64, expected: f64, rel: f64) -> bool {
        (actual - expected).abs() <= expected * rel
    }

    #[test]
    fn known_city_pairs() {
        let london = deg(51.5074, -0.1278);
        let nyc = deg(40.7128, -74.0060);
        let tokyo = deg(35.6762, 139.6503);
        let sydney = deg(-33.8688, 151.2093);
        let paris = deg(48.8566, 2.3522);
        assert!(within(distance_km(london, nyc).unwrap(), 5570.0, 0.005));
        assert!(within(distance_km(tokyo, sydney).unwrap(), 7823.0, 0.005));
        assert!(within(distance_km(london, paris).unwrap(), 344.0, 0.005));
    }

    #[test]
    fn identical_and_symmetric() {
        let p = deg(12.0, 34.0);
        assert_eq!(distance_km(p, p), Some(0.0));
        let q = deg(-45.0, 100.0);
        assert_eq!(distance_km(p, q), distance_km(q, p));
    }

    #[test]
    fn antipodal_is_half_circumference() {
        let half = PI * EARTH_RADIUS_KM;
        for (a, b) in [
            (deg(0.0, 0.0), deg(0.0, 180.0)),
            (deg(90.0, 0.0), deg(-90.0, 0.0)),
            (deg(37.0, -122.0), deg(-37.0, 58.0)),
        ] {
            assert!(within(distance_km(a, b).unwrap(), half, 1e-9));
        }
    }

    #[test]
    fn seam_crossing_takes_short_way() {
        let d = distance_km(deg(0.0, 179.0), deg(0.0, -179.0)).unwrap();
        assert!(within(d, 2.0 * EARTH_RADIUS_KM * PI / 180.0, 1e-9));
        // Lon outside [-180, 180] is equivalent.
        let e = distance_km(deg(0.0, 179.0), deg(0.0, 181.0)).unwrap();
        assert!(within(e, d, 1e-9));
    }

    #[test]
    fn poles() {
        let d = distance_km(deg(90.0, 0.0), deg(90.0, 123.0)).unwrap();
        assert!(d.abs() < 1e-6);
        let q = distance_km(deg(90.0, 0.0), deg(0.0, 77.0)).unwrap();
        assert!(within(q, PI / 2.0 * EARTH_RADIUS_KM, 1e-9));
    }

    #[test]
    fn non_finite_rejected() {
        let p = deg(0.0, 0.0);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(distance_km((bad, 0.0), p), None);
            assert_eq!(distance_km(p, (0.0, bad)), None);
            assert_eq!(interpolate(p, p, bad), None);
            assert!(sample_arc((bad, bad), p).is_empty());
        }
        assert_eq!(distance_km((2.0, 0.0), p), None); // latitude beyond a pole
    }

    #[test]
    fn interpolation_endpoints_and_midpoint() {
        let a = deg(10.0, 20.0);
        let b = deg(50.0, 80.0);
        let s = interpolate(a, b, 0.0).unwrap();
        let e = interpolate(a, b, 1.0).unwrap();
        assert!(distance_km(s, a).unwrap() < 1e-6);
        assert!(distance_km(e, b).unwrap() < 1e-6);
        let m = interpolate(a, b, 0.5).unwrap();
        let total = distance_km(a, b).unwrap();
        assert!(within(distance_km(a, m).unwrap(), total / 2.0, 1e-9));
        assert!(within(distance_km(m, b).unwrap(), total / 2.0, 1e-9));
    }

    #[test]
    fn interpolation_identical_points() {
        let p = deg(33.0, 44.0);
        assert_eq!(interpolate(p, p, 0.5), Some(p));
        assert_eq!(sample_arc(p, p).len(), 2);
    }

    #[test]
    fn interpolation_antipodal_stays_on_a_half_circle() {
        for (a, b) in [
            (deg(0.0, 0.0), deg(0.0, 180.0)),
            (deg(90.0, 0.0), deg(-90.0, 0.0)),
            (deg(20.0, 30.0), deg(-20.0, -150.0)),
        ] {
            let mut prev = a;
            let mut sum = 0.0;
            for i in 1..=100 {
                let p = interpolate(a, b, i as f64 / 100.0).unwrap();
                assert!(p.0.is_finite() && p.1.is_finite());
                sum += distance_km(prev, p).unwrap();
                prev = p;
            }
            assert!(within(sum, PI * EARTH_RADIUS_KM, 1e-6));
            assert!(distance_km(prev, b).unwrap() < 1e-3);
        }
    }

    #[test]
    fn interpolation_over_the_seam_and_pole() {
        let m = interpolate(deg(0.0, 170.0), deg(0.0, -170.0), 0.5).unwrap();
        assert!(distance_km(m, deg(0.0, 180.0)).unwrap() < 1e-6);
        // Between two points on opposite meridians at the same latitude the
        // path goes over the pole.
        let m = interpolate(deg(80.0, 0.0), deg(80.0, 180.0), 0.5).unwrap();
        assert!(within(m.0.to_degrees(), 90.0, 1e-6)); // asin is ill-conditioned at 1
    }

    #[test]
    fn samples_are_dense_and_ordered() {
        let pts = sample_arc(deg(0.0, 0.0), deg(0.0, 90.0));
        assert_eq!(pts.len(), 181);
        for w in pts.windows(2) {
            assert!(central_angle(w[0], w[1]).unwrap() <= SAMPLE_STEP * 1.0001);
        }
    }

    #[test]
    fn measure_state() {
        let mut m = Measure::default();
        assert!(!m.undo());
        assert_eq!(m.total_km(), 0.0);
        assert!(m.add(0.0, 0.0));
        assert!(!m.add(f64::NAN, 0.0));
        assert_eq!(m.total_km(), 0.0);
        m.add(0.0, 1f64.to_radians());
        m.add(0.0, 2f64.to_radians());
        assert_eq!(m.legs_km().len(), 2);
        assert!(within(m.total_km(), 2.0 * 111.195, 0.001));
        assert!(m.undo());
        assert_eq!(m.points().len(), 2);
        m.active = true;
        m.exit();
        assert!(m.points().is_empty() && !m.active);
    }

    #[test]
    fn measure_caps_points() {
        let mut m = Measure::default();
        for _ in 0..MAX_POINTS {
            assert!(m.add(0.1, 0.1));
        }
        assert!(!m.add(0.1, 0.1));
    }

    #[test]
    fn distance_text() {
        assert_eq!(format_distance(5570.4, Units::Metric), "5,570 km");
        assert_eq!(format_distance(1_234_567.0, Units::Metric), "1,234,567 km");
        assert_eq!(format_distance(0.0, Units::Metric), "0 km");
        assert_eq!(format_distance(0.0, Units::Imperial), "0 mi");
        assert_eq!(format_distance(-0.0, Units::Metric), "0 km");
        assert_eq!(format_distance(-0.0, Units::Imperial), "0 mi");
        assert_eq!(format_distance(0.04, Units::Metric), "0 km");
        assert_eq!(format_distance(0.05, Units::Metric), "0.1 km");
        assert_eq!(format_distance(2.5, Units::Metric), "2.5 km");
        assert_eq!(format_distance(5570.0, Units::Imperial), "3,461 mi");
        assert_eq!(format_distance(f64::NAN, Units::Metric), "—");
        assert_eq!(format_distance(f64::INFINITY, Units::Imperial), "—");
        assert_eq!(format_distance(-1.0, Units::Metric), "—");
    }

    #[test]
    fn single_point_total_has_no_negative_zero() {
        let mut m = Measure::default();
        assert!(m.add(0.3, 0.4));
        let total = m.total_km();
        assert!(total.is_sign_positive());
        for units in [Units::Metric, Units::Imperial] {
            let text = format_distance(total, units);
            assert!(!text.contains('-'), "{text}");
            assert!(text.starts_with("0 "), "{text}");
        }
    }

    fn cam(lat: f64, lon: f64, alt: f64) -> Camera {
        let mut c = Camera::new(lat, lon, alt);
        c.width = 800.0;
        c.height = 600.0;
        c
    }

    #[test]
    fn path_follows_the_ground() {
        let a = deg(10.0, 10.0);
        let b = deg(10.0, 30.0);
        let c = cam(10.0, 20.0, 2.0);
        let runs = screen_path(&c, &[a, b]);
        assert_eq!(runs.len(), 1);
        let first = runs[0][0];
        let sp = c.project(a.0, a.1).unwrap();
        assert!((first.0 - sp.0).abs() < 1e-9 && (first.1 - sp.1).abs() < 1e-9);
        // The arc between same-latitude points bulges poleward (up on screen).
        let mid = runs[0][runs[0].len() / 2];
        assert!(mid.1 < first.1);
    }

    #[test]
    fn hidden_segments_are_not_drawn() {
        let a = deg(0.0, 0.0);
        let b = deg(0.0, 180.0);
        // Looking at lon 0: the far half of the arc is behind the globe.
        let runs = screen_path(&cam(0.0, 0.0, 2.0), &[a, b]);
        let n: usize = runs.iter().map(|r| r.len()).sum();
        assert!(n > 0 && n < sample_arc(a, b).len());
        let far = [deg(0.0, 170.0), deg(0.0, -170.0)];
        assert!(screen_path(&cam(0.0, 0.0, 2.0), &far).is_empty());
        assert_eq!(screen_path(&cam(0.0, 180.0, 2.0), &far).len(), 1);
    }

    #[test]
    fn multi_segment_path_is_one_run() {
        let pts = [deg(0.0, -10.0), deg(0.0, 0.0), deg(0.0, 10.0)];
        let runs = screen_path(&cam(0.0, 0.0, 2.0), &pts);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len(), 41); // 20 samples per leg, shared middle
    }

    #[test]
    fn path_edge_cases() {
        let c = cam(0.0, 0.0, 2.0);
        assert!(screen_path(&c, &[]).is_empty());
        assert!(screen_path(&c, &[deg(0.0, 0.0)]).is_empty());
        assert!(screen_path(&c, &[deg(0.0, 0.0), (f64::NAN, 0.0)]).is_empty());
    }
}
