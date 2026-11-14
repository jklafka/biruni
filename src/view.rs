//! Pure, browser-independent helpers for describing a view: HUD text and
//! the shareable `#lat,lon,altitude_km` URL hash.

use crate::camera::{wrap_angle, Camera, EARTH_RADIUS_KM, MAX_ALT, MIN_ALT};

/// Default view: latitude°, longitude°, altitude in globe radii.
pub const HOME: (f64, f64, f64) = (25.0, 0.0, 2.4);

/// A selectable map style. Add new layers here, to `ALL`, and to the shader.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layer {
    /// Land, lakes and country borders in natural colors.
    #[default]
    Political,
    /// Dark, low-color style that makes borders stand out.
    Monochrome,
}

impl Layer {
    pub const ALL: [Layer; 2] = [Layer::Political, Layer::Monochrome];

    /// Short identifier used in the URL hash and in element ids.
    pub fn id(self) -> &'static str {
        match self {
            Layer::Political => "political",
            Layer::Monochrome => "mono",
        }
    }

    pub fn from_id(id: &str) -> Option<Layer> {
        Layer::ALL.into_iter().find(|l| l.id() == id.trim())
    }

    /// Value of the `u_layer` uniform in `globe.frag`.
    pub fn shader_index(self) -> i32 {
        match self {
            Layer::Political => 0,
            Layer::Monochrome => 1,
        }
    }

    /// The layer after this one, wrapping around.
    pub fn next(self) -> Layer {
        let i = Layer::ALL.iter().position(|l| *l == self).unwrap_or(0);
        Layer::ALL[(i + 1) % Layer::ALL.len()]
    }
}

/// Layer named by `&layer=<id>` in a hash. Unknown or missing means `None`.
pub fn parse_hash_layer(hash: &str) -> Option<Layer> {
    hash.trim_start_matches('#')
        .split('&')
        .skip(1)
        .find_map(|kv| kv.strip_prefix("layer="))
        .and_then(Layer::from_id)
}

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

/// Unit system for the scale bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Units {
    Metric,
    Imperial,
}

impl Units {
    pub fn toggled(self) -> Self {
        match self {
            Units::Metric => Units::Imperial,
            Units::Imperial => Units::Metric,
        }
    }
}

pub const KM_PER_MILE: f64 = 1.609_344;
/// Above this altitude (globe radii) the scale varies too much across the
/// view to be meaningful, so no scale bar is shown.
pub const SCALE_MAX_ALT: f64 = 1.0;

/// A scale bar: its width in CSS pixels and its label, e.g. `50 km`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScaleBar {
    pub px: f64,
    pub label: String,
}

/// Largest 1/2/5 × 10^n not exceeding `max`. `None` if `max` is not a
/// positive finite number.
pub fn nice_length(max: f64) -> Option<f64> {
    if !(max.is_finite() && max > 0.0) {
        return None;
    }
    let mut exp = max.log10().floor() as i32;
    let pow = |e: i32| 10f64.powi(e.abs());
    let scaled = |e: i32| if e >= 0 { max / pow(e) } else { max * pow(e) };
    // Correct for rounding in log10 at exact powers of ten.
    let mut m = scaled(exp);
    if m >= 10.0 {
        exp += 1;
        m = scaled(exp);
    } else if m < 1.0 {
        exp -= 1;
        m = scaled(exp);
    }
    let lead = if m >= 5.0 {
        5.0
    } else if m >= 2.0 {
        2.0
    } else {
        1.0
    };
    Some(if exp >= 0 {
        lead * pow(exp)
    } else {
        lead / pow(exp)
    })
}

/// Scale bar for the ground distance at the view center, at most `max_px`
/// wide. `None` when zoomed out too far for a scale to mean anything
/// (`alt > SCALE_MAX_ALT`) or for non-finite input.
pub fn scale_bar(cam: &Camera, units: Units, max_px: f64) -> Option<ScaleBar> {
    let km_per_px = cam.km_per_pixel();
    if !(cam.alt.is_finite() && cam.alt > 0.0 && cam.alt <= SCALE_MAX_ALT)
        || !(km_per_px.is_finite() && km_per_px > 0.0)
        || !(max_px.is_finite() && max_px > 0.0)
    {
        return None;
    }
    let (per_px, suffix) = match units {
        Units::Metric => (km_per_px, "km"),
        Units::Imperial => (km_per_px / KM_PER_MILE, "mi"),
    };
    let len = nice_length(max_px * per_px)?;
    Some(ScaleBar {
        px: len / per_px,
        label: format!("{len} {suffix}"),
    })
}

/// `#lat,lon,altitude_km` — e.g. `#48.8566,2.3522,800`. A non-default layer
/// is appended as `&layer=<id>`.
pub fn format_view_hash(c: &Camera, layer: Layer) -> String {
    let mut hash = format!(
        "#{:.4},{:.4},{:.0}",
        c.lat.to_degrees(),
        wrap_angle(c.lon).to_degrees(),
        c.alt * EARTH_RADIUS_KM
    );
    if layer != Layer::default() {
        hash.push_str("&layer=");
        hash.push_str(layer.id());
    }
    hash
}

/// Full shareable link for the current view: `page_url` without any
/// existing `#fragment`, plus the view hash. Built from the live camera, so
/// it is current even if the debounced URL hash has not been written yet.
pub fn share_url(page_url: &str, c: &Camera, layer: Layer) -> String {
    let base = page_url.split('#').next().unwrap_or("");
    format!("{base}{}", format_view_hash(c, layer))
}

/// Parse a view hash. Altitude is optional; out-of-range values are clamped
/// and anything malformed yields `None`.
pub fn parse_view_hash(hash: &str) -> Option<Camera> {
    let view = hash.trim_start_matches('#').split('&').next().unwrap_or("");
    let mut parts = view.split(',').map(|p| p.trim().parse::<f64>());
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
        let hash = format_view_hash(&c, Layer::Political);
        assert_eq!(hash, "#46.8000,8.2000,600");
        let back = parse_view_hash(&hash).unwrap();
        assert!(close(back.lat, c.lat, 1e-6));
        assert!(close(back.lon, c.lon, 1e-6));
        assert!(close(back.alt, c.alt, 1e-6));
    }

    #[test]
    fn layer_ids_round_trip() {
        for l in Layer::ALL {
            assert_eq!(Layer::from_id(l.id()), Some(l));
        }
        assert_eq!(Layer::from_id("nope"), None);
        assert_eq!(Layer::from_id(""), None);
    }

    #[test]
    fn layer_next_cycles_through_all() {
        let mut l = Layer::default();
        for _ in 0..Layer::ALL.len() {
            l = l.next();
        }
        assert_eq!(l, Layer::default());
        assert_ne!(Layer::Political.next(), Layer::Political);
    }

    #[test]
    fn shader_indices_are_unique() {
        let mut seen: Vec<i32> = Layer::ALL.iter().map(|l| l.shader_index()).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), Layer::ALL.len());
    }

    #[test]
    fn hash_carries_layer() {
        let c = Camera::new(46.8, 8.2, 600.0 / EARTH_RADIUS_KM);
        // Default layer keeps the old, shorter URL form.
        assert_eq!(
            format_view_hash(&c, Layer::Political),
            "#46.8000,8.2000,600"
        );
        let hash = format_view_hash(&c, Layer::Monochrome);
        assert_eq!(hash, "#46.8000,8.2000,600&layer=mono");
        assert_eq!(parse_hash_layer(&hash), Some(Layer::Monochrome));
        let back = parse_view_hash(&hash).unwrap();
        assert!(close(back.alt, c.alt, 1e-6));
    }

    #[test]
    fn share_url_replaces_old_fragment() {
        let c = Camera::new(46.8, 8.2, 600.0 / EARTH_RADIUS_KM);
        assert_eq!(
            share_url(
                "https://example.com/maps/#1,2,3&layer=mono",
                &c,
                Layer::Political
            ),
            "https://example.com/maps/#46.8000,8.2000,600"
        );
        assert_eq!(
            share_url("https://example.com/?a=1", &c, Layer::Monochrome),
            "https://example.com/?a=1#46.8000,8.2000,600&layer=mono"
        );
        assert_eq!(
            share_url("https://example.com/#", &c, Layer::Political),
            "https://example.com/#46.8000,8.2000,600"
        );
    }

    #[test]
    fn share_url_round_trips_seam_and_poles() {
        for (lat, lon) in [
            (0.0, 180.0),
            (0.0, -180.0),
            (85.0, 179.9),
            (-85.0, -179.9),
            (90.0, 0.0),
        ] {
            let c = Camera::new(lat, lon, 400.0 / EARTH_RADIUS_KM);
            let url = share_url("https://x.test/", &c, Layer::Monochrome);
            let hash = &url[url.find('#').unwrap()..];
            let back = parse_view_hash(hash).expect("parses");
            assert!(back.lat.abs() <= 85f64.to_radians() + 1e-9);
            // Longitude agrees modulo the seam.
            assert!(wrap_angle(back.lon - c.lon).abs() < 1e-3, "{url}");
            assert_eq!(parse_hash_layer(hash), Some(Layer::Monochrome));
        }
    }

    #[test]
    fn hash_layer_edge_cases() {
        assert_eq!(parse_hash_layer("#1,2,3"), None);
        assert_eq!(parse_hash_layer("#1,2,3&layer=bogus"), None);
        assert_eq!(
            parse_hash_layer("#1,2&foo=1&layer=mono"),
            Some(Layer::Monochrome)
        );
        assert_eq!(parse_hash_layer(""), None);
        assert!(parse_view_hash("#1,2&layer=mono").is_some());
        assert!(parse_view_hash("#&layer=mono").is_none());
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

    fn sized(lat: f64, alt: f64, w: f64, h: f64) -> Camera {
        let mut c = Camera::new(lat, 10.0, alt);
        c.width = w;
        c.height = h;
        c
    }

    #[test]
    fn nice_lengths() {
        assert_eq!(nice_length(1.0), Some(1.0));
        assert_eq!(nice_length(1.9), Some(1.0));
        assert_eq!(nice_length(2.0), Some(2.0));
        assert_eq!(nice_length(4.99), Some(2.0));
        assert_eq!(nice_length(5.0), Some(5.0));
        assert_eq!(nice_length(99.0), Some(50.0));
        assert_eq!(nice_length(100.0), Some(100.0));
        assert_eq!(nice_length(1000.0), Some(1000.0));
        assert_eq!(nice_length(0.35), Some(0.2));
        assert_eq!(nice_length(0.05), Some(0.05));
        assert_eq!(nice_length(0.001), Some(0.001));
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(nice_length(bad), None);
        }
    }

    #[test]
    fn scale_bar_matches_ground_distance() {
        for alt in [0.06, 0.1, 0.3, 0.7, 1.0] {
            let c = sized(30.0, alt, 1200.0, 800.0);
            for units in [Units::Metric, Units::Imperial] {
                let bar = scale_bar(&c, units, 120.0).unwrap();
                assert!(bar.px <= 120.0 + 1e-9 && bar.px >= 120.0 / 2.5, "{bar:?}");
                // Bar length in km must equal px * km/px, and be a nice number.
                let value: f64 = bar.label.split(' ').next().unwrap().parse().unwrap();
                let km = match units {
                    Units::Metric => value,
                    Units::Imperial => value * KM_PER_MILE,
                };
                assert!((bar.px * c.km_per_pixel() - km).abs() / km < 1e-9);
                assert_eq!(nice_length(value), Some(value));
            }
        }
    }

    #[test]
    fn scale_bar_agrees_with_picking() {
        // Two pixels apart at the center should be ~km_per_pixel apart on the ground.
        let c = sized(20.0, 0.1, 1000.0, 700.0);
        let (la1, lo1) = c.pick(500.0, 350.0).unwrap();
        let (la2, lo2) = c.pick(600.0, 350.0).unwrap();
        let (p, q) = (
            crate::camera::spherical(la1, lo1),
            crate::camera::spherical(la2, lo2),
        );
        let chord = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
        let km = chord * EARTH_RADIUS_KM;
        assert!((km / 100.0 - c.km_per_pixel()).abs() / c.km_per_pixel() < 0.03);
    }

    #[test]
    fn scale_bar_known_value_and_units() {
        // alt 0.1, 800 px shorter side: 0.1*0.36397/400*6371 = 0.5797 km/px.
        let c = sized(0.0, 0.1, 1200.0, 800.0);
        let m = scale_bar(&c, Units::Metric, 100.0).unwrap();
        assert_eq!(m.label, "50 km");
        let i = scale_bar(&c, Units::Imperial, 100.0).unwrap();
        assert_eq!(i.label, "20 mi");
    }

    #[test]
    fn scale_bar_uses_shorter_axis_and_ignores_latitude() {
        let a = scale_bar(&sized(0.0, 0.2, 1200.0, 800.0), Units::Metric, 100.0);
        let b = scale_bar(&sized(0.0, 0.2, 800.0, 1200.0), Units::Metric, 100.0);
        assert_eq!(a, b);
        for lat in [-85.0, 0.0, 85.0] {
            let c = scale_bar(&sized(lat, 0.2, 1200.0, 800.0), Units::Metric, 100.0);
            assert_eq!(c, a);
        }
    }

    #[test]
    fn scale_bar_hidden_when_zoomed_out() {
        assert!(scale_bar(
            &sized(0.0, SCALE_MAX_ALT, 800.0, 600.0),
            Units::Metric,
            100.0
        )
        .is_some());
        assert!(scale_bar(&sized(0.0, 1.01, 800.0, 600.0), Units::Metric, 100.0).is_none());
        assert!(scale_bar(&sized(0.0, 2.4, 800.0, 600.0), Units::Metric, 100.0).is_none());
        assert!(scale_bar(&sized(0.0, MAX_ALT, 800.0, 600.0), Units::Metric, 100.0).is_none());
    }

    #[test]
    fn scale_bar_closest_zoom_and_tiny_viewport() {
        let c = sized(0.0, MIN_ALT, 800.0, 600.0);
        let bar = scale_bar(&c, Units::Metric, 100.0).unwrap();
        assert!(bar.px > 0.0 && bar.px <= 100.0);
        // Degenerate viewports must not panic.
        assert!(scale_bar(&sized(0.0, MIN_ALT, 0.0, 0.0), Units::Metric, 100.0).is_some());
    }

    #[test]
    fn scale_bar_rejects_non_finite() {
        let ok = sized(0.0, 0.2, 800.0, 600.0);
        for bad in [f64::NAN, f64::INFINITY, -1.0, 0.0] {
            assert!(scale_bar(&ok, Units::Metric, bad).is_none());
            assert!(scale_bar(&sized(0.0, bad, 800.0, 600.0), Units::Metric, 100.0).is_none());
        }
        let mut c = ok;
        c.width = f64::NAN;
        c.height = f64::NAN;
        let _ = scale_bar(&c, Units::Imperial, 100.0); // must not panic
    }

    #[test]
    fn units_toggle() {
        assert_eq!(Units::Metric.toggled(), Units::Imperial);
        assert_eq!(Units::Imperial.toggled().toggled(), Units::Imperial);
    }
}
