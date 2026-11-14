//! Pure, browser-independent helpers for describing a view: HUD text and
//! the shareable `#lat,lon,altitude_km` URL hash.

use crate::camera::{wrap_angle, Camera, EARTH_RADIUS_KM, MAX_ALT, MIN_ALT};

/// Default view: latitude°, longitude°, altitude in globe radii.
pub const HOME: (f64, f64, f64) = (25.0, 0.0, 2.4);

/// Latitude/longitude (radians) as e.g. `48.86° N  2.35° E`.
pub fn format_lat_lon(lat: f64, lon: f64) -> String {
    let (lat, lon) = (lat.to_degrees(), wrap_angle(lon).to_degrees());
    format!(
        "{:.2}° {}  {:.2}° {}",
        lat.abs(),
        if lat >= 0.0 { 'N' } else { 'S' },
        lon.abs(),
        if lon >= 0.0 { 'E' } else { 'W' }
    )
}

/// Altitude in globe radii as whole kilometres with thousands separators.
pub fn format_altitude(alt: f64) -> String {
    let km = (alt * EARTH_RADIUS_KM).round().max(0.0) as u64;
    let digits = km.to_string();
    let mut out = String::with_capacity(digits.len() + 4);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out.push_str(" km");
    out
}

/// `#lat,lon,altitude_km` — e.g. `#48.8566,2.3522,800`.
pub fn format_view_hash(c: &Camera) -> String {
    format!(
        "#{:.4},{:.4},{:.0}",
        c.lat.to_degrees(),
        wrap_angle(c.lon).to_degrees(),
        c.alt * EARTH_RADIUS_KM
    )
}

/// Parse a view hash. Altitude is optional; out-of-range values are clamped
/// and anything malformed yields `None`.
pub fn parse_view_hash(hash: &str) -> Option<Camera> {
    let mut parts = hash
        .trim_start_matches('#')
        .split(',')
        .map(|p| p.trim().parse::<f64>());
    let lat = parts.next()?.ok()?;
    let lon = parts.next()?.ok()?;
    let alt_km = match parts.next() {
        Some(v) => v.ok()?,
        None => HOME.2 * EARTH_RADIUS_KM,
    };
    if parts.next().is_some() || !(lat.is_finite() && lon.is_finite() && alt_km.is_finite()) {
        return None;
    }
    Some(Camera::new(
        lat.clamp(-85.0, 85.0),
        wrap_angle(lon.to_radians()).to_degrees(),
        (alt_km / EARTH_RADIUS_KM).clamp(MIN_ALT, MAX_ALT),
    ))
}

/// Tracks what the next frame must redo. Moving the cursor over the globe
/// only changes the HUD readout, so it must not trigger a full ray-trace.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RedrawFlags {
    render: bool,
    hud: bool,
}

impl RedrawFlags {
    /// Start dirty so the first frame draws.
    pub fn new() -> Self {
        Self {
            render: true,
            hud: true,
        }
    }

    /// The camera, canvas size or map texture changed.
    pub fn mark_render(&mut self) {
        self.render = true;
    }

    /// Only HUD text may have changed.
    pub fn mark_hud(&mut self) {
        self.hud = true;
    }

    /// Returns `(draw, update_hud)` for this frame and clears the flags.
    /// A redraw always refreshes the HUD too.
    pub fn take(&mut self) -> (bool, bool) {
        let out = (self.render, self.render || self.hud);
        *self = Self::default();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn lat_lon_hemispheres() {
        assert_eq!(
            format_lat_lon(48.8566f64.to_radians(), 2.3522f64.to_radians()),
            "48.86° N  2.35° E"
        );
        assert_eq!(
            format_lat_lon((-33.8688f64).to_radians(), (-70.6693f64).to_radians()),
            "33.87° S  70.67° W"
        );
    }

    #[test]
    fn lat_lon_wraps_longitude() {
        assert_eq!(
            format_lat_lon(0.0, 190f64.to_radians()),
            "0.00° N  170.00° W"
        );
    }

    #[test]
    fn altitude_formatting() {
        assert_eq!(format_altitude(0.0), "0 km");
        assert_eq!(format_altitude(400.0 / EARTH_RADIUS_KM), "400 km");
        assert_eq!(format_altitude(1500.0 / EARTH_RADIUS_KM), "1,500 km");
        assert_eq!(format_altitude(2.4), "15,290 km");
        assert_eq!(
            format_altitude(1_234_567.0 / EARTH_RADIUS_KM),
            "1,234,567 km"
        );
    }

    #[test]
    fn altitude_rounds_before_grouping() {
        // 15,999.7 km must not render as "15,1000 km".
        assert_eq!(format_altitude(15_999.7 / EARTH_RADIUS_KM), "16,000 km");
        assert_eq!(format_altitude(999.6 / EARTH_RADIUS_KM), "1,000 km");
    }

    #[test]
    fn hash_round_trip() {
        let c = Camera::new(46.8, 8.2, 600.0 / EARTH_RADIUS_KM);
        let hash = format_view_hash(&c);
        assert_eq!(hash, "#46.8000,8.2000,600");
        let back = parse_view_hash(&hash).unwrap();
        assert!(close(back.lat, c.lat, 1e-6));
        assert!(close(back.lon, c.lon, 1e-6));
        assert!(close(back.alt, c.alt, 1e-6));
    }

    #[test]
    fn hash_without_altitude_uses_home_altitude() {
        let c = parse_view_hash("#10,20").unwrap();
        assert!(close(c.alt, HOME.2, 1e-9));
    }

    #[test]
    fn hash_tolerates_whitespace_and_missing_hash_sign() {
        let c = parse_view_hash(" 10 , -20 , 800 ").unwrap();
        assert!(close(c.lat.to_degrees(), 10.0, 1e-9));
        assert!(close(c.lon.to_degrees(), -20.0, 1e-9));
    }

    #[test]
    fn hash_clamps_out_of_range_values() {
        let c = parse_view_hash("#95,540,1").unwrap();
        assert!(close(c.lat.to_degrees(), 85.0, 1e-9));
        assert!(close(c.lon.to_degrees().abs(), 180.0, 1e-9));
        assert!(close(c.alt, MIN_ALT, 1e-12));
        let far = parse_view_hash("#0,0,1e9").unwrap();
        assert!(close(far.alt, MAX_ALT, 1e-12));
    }

    #[test]
    fn hash_rejects_malformed_input() {
        for bad in [
            "",
            "#",
            "#abc",
            "#10",
            "#10,x",
            "#10,20,km",
            "#NaN,0",
            "#0,inf",
            "#1,2,3,4",
        ] {
            assert!(parse_view_hash(bad).is_none(), "accepted {bad:?}");
        }
    }

    #[test]
    fn cursor_move_updates_hud_without_redraw() {
        let mut f = RedrawFlags::default();
        f.mark_hud();
        assert_eq!(f.take(), (false, true));
        assert_eq!(f.take(), (false, false));
    }

    #[test]
    fn render_change_redraws_and_refreshes_hud() {
        let mut f = RedrawFlags::default();
        f.mark_render();
        assert_eq!(f.take(), (true, true));
        assert_eq!(f.take(), (false, false));
    }

    #[test]
    fn first_frame_draws() {
        assert_eq!(RedrawFlags::new().take(), (true, true));
    }
}
