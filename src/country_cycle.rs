//! Keyboard cycling through the countries visible on screen. Pure and
//! browser-free, so it is unit tested natively.
//!
//! A country counts as visible when its anchor point (see
//! `Countries::anchors`) projects onto the viewport: in front of the horizon
//! on the globe, inside the map outline on a flat projection, and within the
//! canvas. Visible countries are ordered in reading order: top to bottom in
//! horizontal bands, left to right inside a band. Bands (about a sixth of the
//! viewport height) keep countries at nearly the same height together, so
//! Tab walks a row of neighbours instead of zig-zagging on a few pixels of
//! difference in y. Reading order is predictable and works the same on every
//! projection; ordering by size would reshuffle the list as you zoom.

use crate::camera::Camera;

/// One anchor per country: (index, lat degrees, lon degrees).
pub type Anchor = (usize, f64, f64);

/// Bands per viewport height.
const BANDS: f64 = 6.0;

/// Visible countries in reading order. Anchors that are non-finite, hidden or
/// off screen are dropped, as is a degenerate viewport.
pub fn visible_order(anchors: &[Anchor], cam: &Camera) -> Vec<usize> {
    let (w, h) = (cam.width, cam.height);
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return Vec::new();
    }
    let band_h = h / BANDS;
    let mut found: Vec<(i64, f64, usize)> = anchors
        .iter()
        .filter(|(_, lat, lon)| lat.is_finite() && lon.is_finite())
        .filter_map(|&(i, lat, lon)| {
            let (x, y) = cam.project(lat.to_radians(), lon.to_radians())?;
            let on_screen = x.is_finite() && y.is_finite() && (0.0..=w).contains(&x);
            (on_screen && (0.0..=h).contains(&y)).then_some(((y / band_h) as i64, x, i))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
    found.into_iter().map(|(_, _, i)| i).collect()
}

/// Where a step lands.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Step {
    /// Select this country.
    To(usize),
    /// Stepped past an end (no wrap) or nothing is visible: let the key through.
    Release,
}

/// Next or previous country in `order` from `current`. With `wrap`, the ends
/// wrap around; without, stepping past an end gives `Release` so Tab does not
/// trap focus. A `current` that is not in `order` (nothing selected, or it
/// scrolled out of view) starts from the first country going forward and the
/// last going back.
pub fn step(order: &[usize], current: Option<usize>, forward: bool, wrap: bool) -> Step {
    let n = order.len();
    if n == 0 {
        return Step::Release;
    }
    let pos = current.and_then(|c| order.iter().position(|&i| i == c));
    let next = match (pos, forward) {
        (None, true) => 0,
        (None, false) => n - 1,
        (Some(p), true) if p + 1 < n => p + 1,
        (Some(p), false) if p > 0 => p - 1,
        (Some(_), true) if wrap => 0,
        (Some(_), false) if wrap => n - 1,
        _ => return Step::Release,
    };
    Step::To(order[next])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::Projection;

    fn cam(lat: f64, lon: f64, alt: f64, proj: Projection) -> Camera {
        let mut c = Camera::new(lat, lon, alt);
        c.width = 1200.0;
        c.height = 600.0;
        c.set_projection(proj);
        c
    }

    #[test]
    fn globe_horizon_and_reading_order() {
        let c = cam(46.0, 8.0, 1.0, Projection::Globe);
        // Berlin-ish, Paris-ish, Madrid-ish, Sydney (far side), NaN.
        let a: Vec<Anchor> = vec![
            (0, 52.0, 13.0),
            (1, 48.8, 2.3),
            (2, 40.4, -3.7),
            (3, -33.9, 151.0),
            (4, f64::NAN, 0.0),
        ];
        let o = visible_order(&a, &c);
        assert!(!o.contains(&3) && !o.contains(&4));
        let pos = |i| o.iter().position(|&x| x == i);
        // Berlin is north of Madrid: earlier in reading order.
        assert!(pos(0).unwrap() < pos(2).unwrap());
    }

    #[test]
    fn same_band_orders_by_x() {
        let c = cam(0.0, 0.0, 1.0, Projection::Equirectangular);
        let a: Vec<Anchor> = vec![(7, -0.1, 10.0), (3, -0.3, -10.0), (5, -0.2, 0.0)];
        assert_eq!(visible_order(&a, &c), vec![3, 5, 7]);
    }

    #[test]
    fn flat_map_domain_and_offscreen() {
        let mut c = cam(0.0, 0.0, 1.0, Projection::Mercator);
        c.width = 1200.0;
        c.height = 600.0;
        // Mercator has no poles; a far-east anchor may be off screen when zoomed in.
        c.alt = 0.05;
        let a: Vec<Anchor> = vec![(0, 89.9, 0.0), (1, 0.0, 0.0), (2, 0.0, 170.0)];
        let o = visible_order(&a, &c);
        assert_eq!(o, vec![1]);
    }

    #[test]
    fn degenerate_inputs() {
        let mut c = cam(0.0, 0.0, 1.0, Projection::Globe);
        assert!(visible_order(&[], &c).is_empty());
        c.width = f64::NAN;
        assert!(visible_order(&[(0, 0.0, 0.0)], &c).is_empty());
        c.width = 0.0;
        assert!(visible_order(&[(0, 0.0, 0.0)], &c).is_empty());
    }

    #[test]
    fn step_no_wrap_releases_at_ends() {
        let o = [4, 9, 2];
        assert_eq!(step(&o, None, true, false), Step::To(4));
        assert_eq!(step(&o, None, false, false), Step::To(2));
        assert_eq!(step(&o, Some(4), true, false), Step::To(9));
        assert_eq!(step(&o, Some(2), true, false), Step::Release);
        assert_eq!(step(&o, Some(4), false, false), Step::Release);
        assert_eq!(step(&o, Some(9), false, false), Step::To(4));
    }

    #[test]
    fn step_wraps() {
        let o = [4, 9, 2];
        assert_eq!(step(&o, Some(2), true, true), Step::To(4));
        assert_eq!(step(&o, Some(4), false, true), Step::To(2));
    }

    #[test]
    fn step_empty_and_stale_current() {
        assert_eq!(step(&[], None, true, true), Step::Release);
        assert_eq!(step(&[], Some(3), false, false), Step::Release);
        // A selection that is no longer visible restarts at an end.
        assert_eq!(step(&[1, 2], Some(99), true, false), Step::To(1));
        assert_eq!(step(&[1, 2], Some(99), false, false), Step::To(2));
        assert_eq!(step(&[8], Some(8), true, true), Step::To(8));
    }
}
