//! Great-circle measurement: pure, browser-free math and state.
//!
//! Distances use the haversine formula on a sphere of radius
//! `EARTH_RADIUS_KM`, which is within about 0.5% of the WGS84 geodesic.

use crate::camera::{spherical, Camera, EARTH_RADIUS_KM};
use crate::controls::{DOUBLE_CLICK_MS, TAP_SLOP_PX};
use crate::view::{group_thousands, Units};
use std::f64::consts::PI;

/// Most points a path may have.
pub const MAX_POINTS: usize = 100;
/// Angular spacing of samples along a drawn arc, radians (0.5°).
const SAMPLE_STEP: f64 = 0.5 * PI / 180.0;
/// Hard cap on samples per segment.
const MAX_SAMPLES: usize = 720;

/// Screen radius (CSS pixels) within which a press grabs a point, for a
/// mouse and for touch or pen (fingers are fat and imprecise).
pub const HIT_RADIUS_MOUSE_PX: f64 = 11.0;
pub const HIT_RADIUS_TOUCH_PX: f64 = 22.0;
/// Distance in CSS pixels that Shift+arrow moves the selected point.
pub const NUDGE_PX: f64 = 4.0;

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

/// Hit radius in CSS pixels for a pointer: larger for touch and pen.
pub fn hit_radius(coarse: bool) -> f64 {
    if coarse {
        HIT_RADIUS_TOUCH_PX
    } else {
        HIT_RADIUS_MOUSE_PX
    }
}

/// Index of the point nearest to a screen position and within `radius` CSS
/// pixels. Points that are not visible (behind the globe, outside a flat
/// map's domain) never hit. Ties go to the later point, which is drawn on top.
pub fn hit_test(
    cam: &Camera,
    points: &[(f64, f64)],
    sx: f64,
    sy: f64,
    radius: f64,
) -> Option<usize> {
    if !(sx.is_finite() && sy.is_finite() && radius.is_finite()) {
        return None;
    }
    let mut best: Option<(usize, f64)> = None;
    for (i, p) in points.iter().enumerate() {
        let Some((x, y)) = cam.project(p.0, p.1) else {
            continue;
        };
        if !(x.is_finite() && y.is_finite()) {
            continue;
        }
        let d = (x - sx).hypot(y - sy);
        if d <= radius && best.is_none_or(|(_, bd)| d <= bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

/// A press on a point that may become a drag.
#[derive(Clone, Copy, Debug)]
struct Grab {
    id: i32,
    idx: usize,
    start: (f64, f64),
    /// Screen offset from the pointer to the point's center when grabbed,
    /// so the point does not jump under the cursor.
    offset: (f64, f64),
    moved: bool,
}

/// What a release did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    /// Not our pointer.
    Ignored,
    /// A drag ended or a click only selected the point.
    Done,
    /// A second click on the same point removed it.
    Removed,
}

/// A path of points on the globe being measured.
#[derive(Clone, Debug, Default)]
pub struct Measure {
    pub active: bool,
    points: Vec<(f64, f64)>,
    selected: Option<usize>,
    hover: Option<usize>,
    grab: Option<Grab>,
    /// Last click on a point: index and time, for double-click removal.
    last_click: Option<(usize, f64)>,
}

impl Measure {
    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn hover(&self) -> Option<usize> {
        self.hover
    }

    /// Add a point (radians). Returns false if it was rejected: non-finite,
    /// or the path is full. Adding clears the selection.
    pub fn add(&mut self, lat: f64, lon: f64) -> bool {
        if !is_valid((lat, lon)) || self.points.len() >= MAX_POINTS {
            return false;
        }
        self.points.push((lat, lon));
        self.selected = None;
        true
    }

    /// Remove the selected point, or the last one when none is selected.
    /// Returns false if there was nothing to remove.
    pub fn undo(&mut self) -> bool {
        match self.selected {
            Some(i) => self.remove(i),
            None => match self.points.len().checked_sub(1) {
                Some(last) => self.remove(last),
                None => false,
            },
        }
    }

    /// Remove the point at `i`; the legs on either side merge into one.
    /// Selection, hover and any drag are fixed up. False if out of range.
    pub fn remove(&mut self, i: usize) -> bool {
        if i >= self.points.len() {
            return false;
        }
        self.points.remove(i);
        self.selected = match self.selected {
            Some(j) if j == i => None,
            Some(j) if j > i => Some(j - 1),
            other => other,
        };
        self.hover = None;
        self.last_click = None;
        self.grab = None;
        true
    }

    /// Remove only the selected point. False if none is selected.
    pub fn remove_selected(&mut self) -> bool {
        match self.selected {
            Some(i) => self.remove(i),
            None => false,
        }
    }

    /// Select a point (out-of-range clears the selection). True on change.
    pub fn select(&mut self, i: Option<usize>) -> bool {
        let i = i.filter(|&i| i < self.points.len());
        let changed = self.selected != i;
        self.selected = i;
        changed
    }

    /// Set the hovered point. True on change.
    pub fn set_hover(&mut self, i: Option<usize>) -> bool {
        let i = i.filter(|&i| i < self.points.len());
        let changed = self.hover != i;
        self.hover = i;
        changed
    }

    /// Move a point. False if `i` is out of range or the position invalid.
    pub fn move_point(&mut self, i: usize, lat: f64, lon: f64) -> bool {
        if !is_valid((lat, lon)) {
            return false;
        }
        match self.points.get_mut(i) {
            Some(p) => {
                *p = (lat, lon);
                true
            }
            None => false,
        }
    }

    /// Keyboard cycling that wraps (`[` and `]`): next or previous point;
    /// with nothing selected, next starts at the first and previous at the
    /// last. False when there are no points.
    pub fn cycle_selection(&mut self, forward: bool) -> bool {
        let n = self.points.len();
        if n == 0 {
            return false;
        }
        self.selected = Some(match (self.selected, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        });
        true
    }

    /// Tab / Shift+Tab: like `cycle_selection` but it does not wrap. Stepping
    /// past either end clears the selection and returns false, so the key is
    /// left alone and focus can move on instead of being trapped.
    pub fn tab_selection(&mut self, forward: bool) -> bool {
        let n = self.points.len();
        let next = match (self.selected, forward) {
            _ if n == 0 => None,
            (None, true) => Some(0),
            (None, false) => Some(n - 1),
            (Some(i), true) if i + 1 < n => Some(i + 1),
            (Some(i), false) if i > 0 => Some(i - 1),
            _ => None,
        };
        self.selected = next;
        next.is_some()
    }

    /// Move the selected point by a screen offset (CSS pixels). False when
    /// nothing is selected, the point is not visible, or the target is off
    /// the globe or map.
    pub fn nudge_selected(&mut self, cam: &Camera, dx: f64, dy: f64) -> bool {
        let Some(i) = self.selected else {
            return false;
        };
        let Some(p) = self.points.get(i) else {
            return false;
        };
        let Some((x, y)) = cam.project(p.0, p.1) else {
            return false;
        };
        match cam.pick(x + dx, y + dy) {
            Some((lat, lon)) => self.move_point(i, lat, lon),
            None => false,
        }
    }

    /// A pointer went down. If it hits a point, that point is selected and
    /// the press belongs to the editor: returns true and the caller must not
    /// pan. While a grab is active, other pointers (a second finger) are
    /// also swallowed.
    pub fn press(&mut self, cam: &Camera, id: i32, x: f64, y: f64, radius: f64) -> bool {
        if self.grab.is_some() {
            return true;
        }
        let Some(idx) = hit_test(cam, &self.points, x, y, radius) else {
            return false;
        };
        let p = self.points[idx];
        let (px, py) = cam.project(p.0, p.1).unwrap_or((x, y));
        self.selected = Some(idx);
        self.grab = Some(Grab {
            id,
            idx,
            start: (x, y),
            offset: (px - x, py - y),
            moved: false,
        });
        true
    }

    /// A pointer moved. Returns true if the path changed. Movement under the
    /// tap slop does nothing, so a click does not nudge the point.
    pub fn drag(&mut self, cam: &Camera, id: i32, x: f64, y: f64) -> bool {
        let Some(mut g) = self.grab.filter(|g| g.id == id) else {
            return false;
        };
        if !(x.is_finite() && y.is_finite()) {
            return false;
        }
        if !g.moved && (x - g.start.0).hypot(y - g.start.1) < TAP_SLOP_PX {
            return false;
        }
        g.moved = true;
        self.grab = Some(g);
        match cam.pick(x + g.offset.0, y + g.offset.1) {
            Some((lat, lon)) => self.move_point(g.idx, lat, lon),
            None => false, // off the globe or map: hold the last position
        }
    }

    /// Whether this pointer is the one dragging a point.
    pub fn owns(&self, id: i32) -> bool {
        self.grab.is_some_and(|g| g.id == id)
    }

    /// A pointer was released at time `t` (ms). A click without a drag
    /// selects the point; a second click on the same point within the
    /// double-click interval removes it.
    pub fn release(&mut self, id: i32, t: f64, cancel: bool) -> Release {
        let Some(g) = self.grab.filter(|g| g.id == id) else {
            return Release::Ignored;
        };
        self.grab = None;
        if g.moved || cancel {
            self.last_click = None;
            return Release::Done;
        }
        match self.last_click {
            Some((i, t0)) if i == g.idx && t >= t0 && t - t0 <= DOUBLE_CLICK_MS => {
                self.remove(g.idx);
                Release::Removed
            }
            _ => {
                self.last_click = Some((g.idx, t));
                Release::Done
            }
        }
    }

    /// Drop all points and leave measure mode.
    pub fn exit(&mut self) {
        *self = Self::default();
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

/// Distance as text with thousands separators, e.g. `5,570 km`,
/// `3,461 mi` or `3,008 nmi`. Below 10 shows one decimal, and zero shows
/// `0 km`; non-finite shows an em dash.
pub fn format_distance(km: f64, units: Units) -> String {
    if !km.is_finite() || km < 0.0 {
        return "—".into();
    }
    let (v, unit) = (km / units.km_per_unit(), units.suffix());
    if v < 10.0 {
        // Zero (including -0.0 from `0.0 * -1.0` style arithmetic) and
        // values that round to 0.0 print as a plain "0".
        if (v * 10.0).round() == 0.0 {
            return format!("0 {unit}");
        }
        return format!("{v:.1} {unit}");
    }
    format!("{} {unit}", group_thousands(v.round() as u64))
}

/// Screen-space polylines (CSS pixels) for the path. Each is a run of
/// consecutive samples that face the camera, so segments hidden behind the
/// globe are not drawn.
pub fn screen_path(cam: &Camera, points: &[(f64, f64)]) -> Vec<Vec<(f64, f64)>> {
    let mut runs: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut cur: Vec<(f64, f64)> = Vec::new();
    let mut joined = false; // `cur` ends at the previous segment's last sample
                            // On a flat map a path that crosses the side edge jumps by about a world
                            // width between two samples; break the line there instead of drawing a
                            // stroke across the map.
    let seam_jump = if cam.is_flat() {
        0.5 * cam.flat_world_width_px()
    } else {
        f64::INFINITY
    };
    for w in points.windows(2) {
        for (i, p) in sample_arc(w[0], w[1]).into_iter().enumerate() {
            if i == 0 && joined {
                continue; // repeats the previous segment's last sample
            }
            let s = cam.project(p.0, p.1);
            if let (Some(s), Some(prev)) = (s, cur.last()) {
                if (s.0 - prev.0).abs() > seam_jump {
                    if cur.len() > 1 {
                        runs.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                }
            }
            match s {
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
    use crate::projection::Projection;

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
        assert_eq!(format_distance(0.0, Units::Nautical), "0 nmi");
        assert_eq!(format_distance(-0.0, Units::Nautical), "0 nmi");
        assert_eq!(format_distance(0.09, Units::Nautical), "0 nmi");
        assert_eq!(format_distance(1.852, Units::Nautical), "1.0 nmi");
        assert_eq!(format_distance(5570.0, Units::Nautical), "3,008 nmi");
        assert_eq!(format_distance(f64::NAN, Units::Nautical), "—");
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
        for units in [Units::Metric, Units::Imperial, Units::Nautical] {
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

    #[test]
    fn flat_path_follows_the_ground_and_breaks_at_the_seam() {
        let mut c = cam(0.0, 0.0, 6.0);
        c.set_projection(Projection::Equirectangular);
        // An ordinary leg is one run whose ends sit on the projected points.
        let a = deg(10.0, -20.0);
        let b = deg(30.0, 40.0);
        let runs = screen_path(&c, &[a, b]);
        assert_eq!(runs.len(), 1);
        let sa = c.project(a.0, a.1).unwrap();
        assert!((runs[0][0].0 - sa.0).abs() < 1e-9 && (runs[0][0].1 - sa.1).abs() < 1e-9);
        // A leg over the antimeridian, with the view centered on it, would
        // otherwise be drawn as a stroke across the whole map.
        let (w, e) = (deg(10.0, 170.0), deg(20.0, -170.0));
        let runs = screen_path(&c, &[w, e]);
        let world = c.flat_world_width_px();
        for run in &runs {
            for pair in run.windows(2) {
                assert!(
                    (pair[1].0 - pair[0].0).abs() < 0.5 * world,
                    "no seam-crossing stroke"
                );
            }
        }
        // Centered on the seam it stays one continuous run.
        let mut c2 = cam(0.0, 180.0, 6.0);
        c2.set_projection(Projection::Mercator);
        let runs = screen_path(&c2, &[w, e]);
        assert_eq!(runs.len(), 1);
        // Mercator drops the part of a polar path it cannot show.
        let runs = screen_path(&c2, &[deg(80.0, 170.0), deg(89.0, 180.0)]);
        assert!(runs.iter().all(|r| r.len() > 1));
        assert!(screen_path(&c2, &[deg(89.0, 0.0), deg(88.0, 10.0)]).is_empty());
    }

    // ---- editing --------------------------------------------------------

    fn three() -> Measure {
        let mut m = Measure::default();
        for (la, lo) in [(0.0_f64, -20.0_f64), (0.0, 0.0), (0.0, 20.0)] {
            assert!(m.add(la, lo.to_radians()));
        }
        m
    }

    fn near(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    #[test]
    fn hit_test_picks_nearest_within_radius() {
        let c = cam(0.0, 0.0, 2.0);
        let m = three();
        let (x, y) = c.project(m.points()[1].0, m.points()[1].1).unwrap();
        assert_eq!(hit_test(&c, m.points(), x, y, 11.0), Some(1));
        assert_eq!(hit_test(&c, m.points(), x + 10.0, y, 11.0), Some(1));
        assert_eq!(hit_test(&c, m.points(), x + 12.0, y, 11.0), None);
        // Touch radius is larger.
        assert!(hit_radius(true) > hit_radius(false));
        assert_eq!(hit_test(&c, m.points(), x + 12.0, y + 12.0, 22.0), Some(1));
        // Overlapping points: the later one wins a tie.
        let dup = [m.points()[1], m.points()[1]];
        assert_eq!(hit_test(&c, &dup, x, y, 11.0), Some(1));
    }

    #[test]
    fn hit_test_ignores_hidden_and_bad_input() {
        let c = cam(0.0, 0.0, 2.0);
        let far = [deg(0.0, 180.0)]; // behind the globe
        let (x, y) = (400.0, 300.0);
        assert_eq!(hit_test(&c, &far, x, y, 1e6), None);
        let m = three();
        assert_eq!(hit_test(&c, m.points(), f64::NAN, y, 11.0), None);
        assert_eq!(hit_test(&c, m.points(), x, f64::INFINITY, 11.0), None);
        assert_eq!(hit_test(&c, m.points(), x, y, f64::NAN), None);
        assert_eq!(hit_test(&c, &[(f64::NAN, 0.0)], x, y, 1e6), None);
    }

    #[test]
    fn hit_test_on_flat_maps_and_across_the_seam() {
        let mut c = cam(0.0, 180.0, 6.0);
        c.set_projection(Projection::Robinson);
        // Points on both sides of the seam project next to each other.
        let pts = [deg(10.0, 179.0), deg(10.0, -179.0)];
        for (i, p) in pts.iter().enumerate() {
            let (x, y) = c.project(p.0, p.1).unwrap();
            assert_eq!(hit_test(&c, &pts, x, y, 5.0), Some(i));
        }
        // Mercator has no pole: a point there is not in the domain.
        c.set_projection(Projection::Mercator);
        let pole = [deg(89.9, 0.0)];
        assert_eq!(hit_test(&c, &pole, 400.0, 0.0, 1e6), None);
    }

    #[test]
    fn dragging_moves_the_point_and_updates_legs() {
        let c = cam(0.0, 0.0, 2.0);
        let mut m = three();
        let before = m.total_km();
        let (x, y) = c.project(m.points()[1].0, m.points()[1].1).unwrap();
        assert!(m.press(&c, 7, x + 3.0, y, 11.0));
        assert_eq!(m.selected(), Some(1));
        // Under the tap slop nothing moves.
        assert!(!m.drag(&c, 7, x + 3.0 + 2.0, y));
        assert!(near(m.points()[1], deg(0.0, 0.0)));
        // Past the slop it follows the cursor, keeping the grab offset.
        assert!(m.drag(&c, 7, x + 3.0, y - 60.0));
        let want = c.pick(x, y - 60.0).unwrap();
        assert!(near(m.points()[1], want));
        assert!(m.total_km() > before, "the path got longer");
        assert_eq!(m.points().len(), 3);
        assert_eq!(m.legs_km().len(), 2);
        // Other pointers cannot move it; ending the drag does not remove.
        assert!(!m.drag(&c, 8, x, y));
        assert_eq!(m.release(7, 100.0, false), Release::Done);
        assert!(!m.owns(7));
        // A drag off the globe holds the last position.
        let held = m.points()[1];
        assert!(m.press(&c, 7, x, y - 60.0, 11.0));
        assert!(!m.drag(&c, 7, 5000.0, 5000.0));
        assert_eq!(m.points()[1], held);
    }

    #[test]
    fn press_elsewhere_is_not_ours() {
        let c = cam(0.0, 0.0, 2.0);
        let mut m = three();
        assert!(!m.press(&c, 1, 10.0, 10.0, 11.0));
        assert!(!m.owns(1));
        assert_eq!(m.release(1, 0.0, false), Release::Ignored);
        // A second finger is swallowed during a grab.
        let (x, y) = c.project(m.points()[0].0, m.points()[0].1).unwrap();
        assert!(m.press(&c, 1, x, y, 11.0));
        assert!(m.press(&c, 2, 10.0, 10.0, 11.0));
        assert!(!m.owns(2));
    }

    #[test]
    fn double_click_on_a_point_removes_it() {
        let c = cam(0.0, 0.0, 2.0);
        let mut m = three();
        let (x, y) = c.project(m.points()[1].0, m.points()[1].1).unwrap();
        assert!(m.press(&c, 1, x, y, 11.0));
        assert_eq!(m.release(1, 1000.0, false), Release::Done);
        assert_eq!(m.points().len(), 3);
        assert!(m.press(&c, 1, x, y, 11.0));
        assert_eq!(m.release(1, 1200.0, false), Release::Removed);
        assert_eq!(m.points().len(), 2);
        assert_eq!(m.selected(), None);
        // Slow second click, or a drag in between: no removal.
        let (x0, y0) = c.project(m.points()[0].0, m.points()[0].1).unwrap();
        m.press(&c, 1, x0, y0, 11.0);
        m.release(1, 5000.0, false);
        m.press(&c, 1, x0, y0, 11.0);
        assert_eq!(
            m.release(1, 5000.0 + DOUBLE_CLICK_MS + 1.0, false),
            Release::Done
        );
        assert_eq!(m.points().len(), 2);
        m.press(&c, 1, x0, y0, 11.0);
        m.drag(&c, 1, x0, y0 + 40.0);
        m.release(1, 6000.0, false);
        let (x0, y0) = c.project(m.points()[0].0, m.points()[0].1).unwrap();
        m.press(&c, 1, x0, y0, 11.0);
        assert_eq!(m.release(1, 6100.0, false), Release::Done);
        assert_eq!(m.points().len(), 2);
    }

    #[test]
    fn removing_a_middle_point_joins_its_neighbours() {
        let mut m = three();
        let legs = m.legs_km();
        let ends = distance_km(m.points()[0], m.points()[2]).unwrap();
        m.select(Some(1));
        assert!(m.remove_selected());
        assert_eq!(m.legs_km().len(), 1);
        assert!(within(m.legs_km()[0], ends, 1e-12));
        assert!(within(m.total_km(), legs[0] + legs[1], 1e-9)); // collinear
        assert_eq!(m.selected(), None);
        assert!(!m.remove_selected());
        assert!(!m.remove(5));
    }

    #[test]
    fn removal_keeps_selection_on_the_same_point() {
        let mut m = three();
        m.select(Some(2));
        let kept = m.points()[2];
        assert!(m.remove(0));
        assert_eq!(m.selected(), Some(1));
        assert_eq!(m.points()[1], kept);
    }

    #[test]
    fn undo_removes_selected_else_last() {
        let mut m = three();
        let first = m.points()[0];
        assert!(m.undo());
        assert_eq!(m.points().len(), 2);
        m.select(Some(0));
        assert!(m.undo());
        assert_eq!(m.points().len(), 1);
        assert_ne!(m.points()[0], first);
        assert!(m.undo());
        assert!(!m.undo());
    }

    #[test]
    fn selection_cycles_and_wraps() {
        let mut m = three();
        assert!(m.cycle_selection(true));
        assert_eq!(m.selected(), Some(0));
        m.cycle_selection(true);
        m.cycle_selection(true);
        m.cycle_selection(true);
        assert_eq!(m.selected(), Some(0));
        m.cycle_selection(false);
        assert_eq!(m.selected(), Some(2));
        m.select(None);
        m.cycle_selection(false);
        assert_eq!(m.selected(), Some(2));
        assert!(!Measure::default().cycle_selection(true));
    }

    #[test]
    fn tab_steps_through_then_lets_focus_go() {
        let mut m = three();
        assert!(m.tab_selection(true));
        assert!(m.tab_selection(true));
        assert!(m.tab_selection(true));
        assert_eq!(m.selected(), Some(2));
        assert!(!m.tab_selection(true));
        assert_eq!(m.selected(), None);
        assert!(m.tab_selection(false));
        assert_eq!(m.selected(), Some(2));
        m.select(Some(0));
        assert!(!m.tab_selection(false));
        assert_eq!(m.selected(), None);
        assert!(!Measure::default().tab_selection(true));
    }

    #[test]
    fn select_and_hover_ignore_out_of_range() {
        let mut m = three();
        assert!(m.select(Some(1)));
        assert!(!m.select(Some(1)));
        assert!(m.select(Some(9)));
        assert_eq!(m.selected(), None);
        assert!(m.set_hover(Some(0)));
        assert!(!m.set_hover(Some(0)));
        assert!(m.set_hover(Some(9)));
        m.select(Some(1));
        m.add(0.0, 0.0);
        assert_eq!(m.selected(), None, "adding clears the selection");
    }

    #[test]
    fn nudge_moves_by_screen_pixels() {
        let c = cam(0.0, 0.0, 2.0);
        let mut m = three();
        assert!(!m.nudge_selected(&c, NUDGE_PX, 0.0), "nothing selected");
        m.select(Some(1));
        let (x, y) = c.project(m.points()[1].0, m.points()[1].1).unwrap();
        assert!(m.nudge_selected(&c, NUDGE_PX, -NUDGE_PX));
        let (nx, ny) = c.project(m.points()[1].0, m.points()[1].1).unwrap();
        assert!((nx - x - NUDGE_PX).abs() < 1e-6 && (ny - y + NUDGE_PX).abs() < 1e-6);
        // A point behind the globe cannot be nudged.
        let mut h = Measure::default();
        h.add(0.0, PI);
        h.select(Some(0));
        assert!(!h.nudge_selected(&c, 4.0, 0.0));
        // Nor off the edge of the globe.
        let mut e = Measure::default();
        let edge = c.pick(400.0, 300.0 + 150.0).unwrap();
        e.add(edge.0, edge.1);
        e.select(Some(0));
        assert!(!e.nudge_selected(&c, 5000.0, 0.0));
    }

    #[test]
    fn flat_map_edit_across_the_seam_stays_finite() {
        let mut c = cam(0.0, 180.0, 6.0);
        c.set_projection(Projection::Equirectangular);
        let mut m = Measure::default();
        m.add(0.1, 179.0_f64.to_radians());
        m.add(0.1, (-179.0_f64).to_radians());
        let (x, y) = c.project(m.points()[0].0, m.points()[0].1).unwrap();
        assert!(m.press(&c, 1, x, y, 11.0));
        // Drag the point east over the seam.
        assert!(m.drag(&c, 1, x + 80.0, y));
        let p = m.points()[0];
        assert!(p.0.is_finite() && p.1.is_finite());
        assert!(distance_km(p, m.points()[1]).unwrap().is_finite());
        m.release(1, 10.0, false);
        // Nudge across the seam: no NaN.
        m.select(Some(1));
        for _ in 0..200 {
            m.nudge_selected(&c, 4.0, 0.0);
        }
        assert!(m
            .points()
            .iter()
            .all(|p| p.0.is_finite() && p.1.is_finite()));
        assert!(m.total_km().is_finite());
        assert!(!m.move_point(0, f64::NAN, 0.0));
        assert!(!m.move_point(0, 0.0, f64::INFINITY));
        assert!(!m.move_point(9, 0.0, 0.0));
    }

    #[test]
    fn exit_resets_everything() {
        let mut m = three();
        m.active = true;
        m.select(Some(1));
        m.set_hover(Some(0));
        m.exit();
        assert!(!m.active && m.points().is_empty());
        assert_eq!((m.selected(), m.hover()), (None, None));
    }
}
