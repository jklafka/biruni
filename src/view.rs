//! Pure, browser-independent helpers for describing a view: HUD text and
//! the shareable `#lat,lon,altitude_km` URL hash.

use crate::camera::{wrap_angle, Camera, EARTH_RADIUS_KM, MAX_ALT, MIN_ALT};

/// Default view: latitude°, longitude°, altitude in globe radii.
pub const HOME: (f64, f64, f64) = (25.0, 0.0, 2.4);

/// A selectable map style. Add new layers here, to `ALL`, and to the shader.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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

    /// Data file (relative to the page) this layer needs before it can be
    /// shown, fetched the first time it is selected. Today's layers are
    /// palettes over the base map and need none.
    pub fn data_url(self) -> Option<&'static str> {
        match self {
            Layer::Political | Layer::Monochrome => None,
        }
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

impl crate::layer_loader::Loadable for Layer {
    fn needs_data(self) -> bool {
        self.data_url().is_some()
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

/// Day/night terminator state as carried by the URL hash.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SunView {
    pub on: bool,
    /// Fixed unix time loaded from `&t=`; `None` follows the clock.
    pub pin: Option<f64>,
}

impl SunView {
    /// Read `&sun=` and `&t=` from a hash. `sun` absent or unrecognised
    /// means off; `t` is kept only when it is a finite, non-negative number.
    pub fn from_hash(hash: &str) -> SunView {
        let h = crate::sun::parse_hash_sun(hash);
        SunView {
            on: h.enabled.unwrap_or(false),
            pin: h.time.filter(|t| *t >= 0.0),
        }
    }
}

/// `#lat,lon,altitude_km[,bearing]` — e.g. `#48.8566,2.3522,800`. The
/// bearing is the compass direction at the top of the screen in degrees
/// (0 to 360) and is written only when the view is not facing north, so
/// links to a north-up view stay short and old links keep working. A non-default layer
/// is appended as `&layer=<id>`. When the terminator is on, `&sun=on` is
/// appended, plus `&t=<unix seconds>` only if the time is pinned: a live
/// clock never writes `t`, so the hash does not change every minute. When
/// the terminator is off neither is written (a pin is remembered by the app
/// but not shared).
pub fn format_view_hash(c: &Camera, layer: Layer, sun: SunView) -> String {
    let mut hash = format!(
        "#{:.4},{:.4},{:.0}",
        c.lat.to_degrees(),
        wrap_angle(c.lon).to_degrees(),
        c.alt * EARTH_RADIUS_KM
    );
    let bearing = format!("{:.1}", c.bearing_deg());
    if bearing != "0.0" && bearing != "360.0" {
        hash.push(',');
        hash.push_str(&bearing);
    }
    if layer != Layer::default() {
        hash.push_str("&layer=");
        hash.push_str(layer.id());
    }
    if sun.on {
        hash.push_str("&sun=on");
        if let Some(t) = sun.pin.filter(|t| t.is_finite() && *t >= 0.0) {
            hash.push_str(&format!("&t={t}"));
        }
    }
    hash
}

/// Full shareable link for the current view: `page_url` without any
/// existing `#fragment`, plus the view hash. Built from the live camera, so
/// it is current even if the debounced URL hash has not been written yet.
pub fn share_url(page_url: &str, c: &Camera, layer: Layer, sun: SunView) -> String {
    let base = page_url.split('#').next().unwrap_or("");
    format!("{base}{}", format_view_hash(c, layer, sun))
}

/// Parse a view hash. Altitude and bearing are optional; out-of-range values
/// are clamped (latitude to the poles) or wrapped (longitude, bearing), and
/// anything malformed yields `None`. A nonzero bearing means free mode.
pub fn parse_view_hash(hash: &str) -> Option<Camera> {
    let view = hash.trim_start_matches('#').split('&').next().unwrap_or("");
    let mut parts = view.split(',').map(|p| p.trim().parse::<f64>());
    let lat = parts.next()?.ok()?;
    let lon = parts.next()?.ok()?;
    let alt_km = match parts.next() {
        Some(v) => v.ok()?,
        None => HOME.2 * EARTH_RADIUS_KM,
    };
    let bearing = match parts.next() {
        Some(v) => v.ok()?,
        None => 0.0,
    };
    if parts.next().is_some()
        || !(lat.is_finite() && lon.is_finite() && alt_km.is_finite() && bearing.is_finite())
    {
        return None;
    }
    Some(
        Camera::new(
            lat.clamp(-90.0, 90.0),
            wrap_angle(lon.to_radians()).to_degrees(),
            (alt_km / EARTH_RADIUS_KM).clamp(MIN_ALT, MAX_ALT),
        )
        .with_bearing(bearing),
    )
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
        let hash = format_view_hash(&c, Layer::Political, SunView::default());
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
            format_view_hash(&c, Layer::Political, SunView::default()),
            "#46.8000,8.2000,600"
        );
        let hash = format_view_hash(&c, Layer::Monochrome, SunView::default());
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
                Layer::Political,
                SunView::default()
            ),
            "https://example.com/maps/#46.8000,8.2000,600"
        );
        assert_eq!(
            share_url(
                "https://example.com/?a=1",
                &c,
                Layer::Monochrome,
                SunView::default()
            ),
            "https://example.com/?a=1#46.8000,8.2000,600&layer=mono"
        );
        assert_eq!(
            share_url(
                "https://example.com/#",
                &c,
                Layer::Political,
                SunView::default()
            ),
            "https://example.com/#46.8000,8.2000,600"
        );
    }

    fn cam() -> Camera {
        Camera::new(46.8, 8.2, 600.0 / EARTH_RADIUS_KM)
    }

    #[test]
    fn hash_carries_sun_on_without_time_for_live_clock() {
        let sun = SunView {
            on: true,
            pin: None,
        };
        let hash = format_view_hash(&cam(), Layer::Political, sun);
        assert_eq!(hash, "#46.8000,8.2000,600&sun=on");
        assert!(!hash.contains("t="));
        assert_eq!(SunView::from_hash(&hash), sun);
    }

    #[test]
    fn hash_carries_pinned_time() {
        let sun = SunView {
            on: true,
            pin: Some(1_718_992_800.0),
        };
        let hash = format_view_hash(&cam(), Layer::Political, sun);
        assert_eq!(hash, "#46.8000,8.2000,600&sun=on&t=1718992800");
        assert_eq!(SunView::from_hash(&hash), sun);
    }

    #[test]
    fn hash_omits_sun_and_time_when_off() {
        let sun = SunView {
            on: false,
            pin: Some(1_718_992_800.0),
        };
        let hash = format_view_hash(&cam(), Layer::Political, sun);
        assert_eq!(hash, "#46.8000,8.2000,600");
        assert_eq!(SunView::from_hash(&hash), SunView::default());
    }

    #[test]
    fn hash_layer_and_sun_together() {
        let sun = SunView {
            on: true,
            pin: Some(86_400.0),
        };
        let hash = format_view_hash(&cam(), Layer::Monochrome, sun);
        assert_eq!(hash, "#46.8000,8.2000,600&layer=mono&sun=on&t=86400");
        assert_eq!(parse_hash_layer(&hash), Some(Layer::Monochrome));
        assert_eq!(SunView::from_hash(&hash), sun);
        let back = parse_view_hash(&hash).unwrap();
        assert!(close(back.alt, cam().alt, 1e-6));
    }

    #[test]
    fn share_url_includes_sun() {
        let sun = SunView {
            on: true,
            pin: Some(1_718_992_800.0),
        };
        assert_eq!(
            share_url(
                "https://x.test/#1,2,3&sun=off",
                &cam(),
                Layer::Political,
                sun
            ),
            "https://x.test/#46.8000,8.2000,600&sun=on&t=1718992800"
        );
        let live = SunView {
            on: true,
            pin: None,
        };
        assert_eq!(
            share_url("https://x.test/", &cam(), Layer::Monochrome, live),
            "https://x.test/#46.8000,8.2000,600&layer=mono&sun=on"
        );
    }

    #[test]
    fn sun_hash_rejects_malformed_time_and_ignores_unknown_keys() {
        for bad in ["NaN", "inf", "-inf", "-5", "abc", "", "1e999"] {
            let h = format!("#1,2,3&sun=on&t={bad}");
            assert_eq!(
                SunView::from_hash(&h),
                SunView {
                    on: true,
                    pin: None
                },
                "{bad:?}"
            );
        }
        // Huge but finite is kept, and is never written back as NaN/inf.
        let huge = SunView::from_hash("#1,2,3&sun=on&t=1e300");
        assert_eq!(huge.pin, Some(1e300));
        let unknown = SunView::from_hash("#1,2,3&foo=bar&sun=on&zzz&t=60&x=");
        assert_eq!(
            unknown,
            SunView {
                on: true,
                pin: Some(60.0)
            }
        );
        // A non-finite pin is never formatted.
        let h = format_view_hash(
            &cam(),
            Layer::Political,
            SunView {
                on: true,
                pin: Some(f64::NAN),
            },
        );
        assert_eq!(h, "#46.8000,8.2000,600&sun=on");
        assert_eq!(SunView::from_hash("#1,2,3&sun=maybe"), SunView::default());
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
            let url = share_url("https://x.test/", &c, Layer::Monochrome, SunView::default());
            let hash = &url[url.find('#').unwrap()..];
            let back = parse_view_hash(hash).expect("parses");
            assert!(
                (back.lat - c.lat).abs() < 1e-9,
                "poles are reachable: {url}"
            );
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
        assert!(close(c.lat.to_degrees(), 90.0, 1e-9));
        assert!(close(
            parse_view_hash("#-95,0").unwrap().lat.to_degrees(),
            -90.0,
            1e-9
        ));
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
            "#1,2,3,4,5",
            "#1,2,3,x",
            "#1,2,3,NaN",
            "#1,2,3,inf",
        ] {
            assert!(parse_view_hash(bad).is_none(), "accepted {bad:?}");
        }
    }

    #[test]
    fn hash_bearing_is_optional_and_backward_compatible() {
        let old = parse_view_hash("#46.8,8.2,600").unwrap();
        assert_eq!(old.bearing, 0.0);
        assert!(old.north_up);
        let c = parse_view_hash("#46.8,8.2,600,90").unwrap();
        assert!(close(c.bearing.to_degrees(), 90.0, 1e-9));
        assert!(!c.north_up, "a bearing means free mode");
        let wrapped = parse_view_hash("#0,0,600,-90").unwrap();
        assert!(close(wrapped.bearing_deg(), 270.0, 1e-9));
        let big = parse_view_hash("#0,0,600,450").unwrap();
        assert!(close(big.bearing_deg(), 90.0, 1e-9));
        assert!(parse_view_hash("#0,0,600,0").unwrap().north_up);
        // Other fields still follow the view.
        assert!(parse_view_hash("#1,2,3,4&layer=mono").is_some());
    }

    #[test]
    fn hash_writes_bearing_only_when_not_north() {
        let north = Camera::new(10.0, 20.0, 0.1);
        let h = format_view_hash(&north, Layer::default(), SunView::default());
        assert_eq!(h.matches(',').count(), 2, "{h}");
        let turned = Camera::new(10.0, 20.0, 0.1).with_bearing(-30.0);
        let h = format_view_hash(&turned, Layer::Monochrome, SunView::default());
        assert!(h.contains(",330.0&layer=mono"), "{h}");
    }

    #[test]
    fn hash_round_trips_poles_and_bearings() {
        for (lat, lon, b) in [
            (90.0, 0.0, 0.0),
            (-90.0, 123.0, 0.0),
            (90.0, -45.0, 37.5),
            (-90.0, 179.0, 359.0),
            (30.0, 179.9, 180.0),
            (-12.0, -179.9, 12.3),
        ] {
            let c = Camera::new(lat, lon, 400.0 / EARTH_RADIUS_KM).with_bearing(b);
            let url = share_url("https://x.test/", &c, Layer::default(), SunView::default());
            let back = parse_view_hash(&url[url.find('#').unwrap()..]).expect("parses");
            assert!((back.lat - c.lat).abs() < 1e-6, "{url}");
            assert!(wrap_angle(back.lon - c.lon).abs() < 1e-3, "{url}");
            assert!(
                wrap_angle(back.bearing - c.bearing).abs() < 2e-3,
                "{url}: {} vs {}",
                back.bearing,
                c.bearing
            );
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
