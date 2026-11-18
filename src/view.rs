//! Pure, browser-independent helpers for describing a view: HUD text and
//! the shareable `#lat,lon,altitude_km` URL hash.

use crate::camera::{wrap_angle, Camera, EARTH_RADIUS_KM, MAX_ALT, MIN_ALT};
use crate::projection::Projection;
use crate::thematic::Theme;

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
    /// NASA Blue Marble satellite imagery (fetched on first use).
    Imagery,
    /// Natural Earth shaded relief and sea floor (fetched on first use).
    Terrain,
    /// Countries colored by population (see `thematic`).
    Population,
    /// Countries colored by GDP per capita.
    GdpPerCapita,
    /// Countries colored by income group.
    Income,
}

impl Layer {
    pub const ALL: [Layer; 7] = [
        Layer::Political,
        Layer::Monochrome,
        Layer::Imagery,
        Layer::Terrain,
        Layer::Population,
        Layer::GdpPerCapita,
        Layer::Income,
    ];

    /// The statistic a thematic layer colors countries by.
    pub fn theme(self) -> Option<Theme> {
        match self {
            Layer::Population => Some(Theme::Population),
            Layer::GdpPerCapita => Some(Theme::GdpPerCapita),
            Layer::Income => Some(Theme::Income),
            _ => None,
        }
    }

    /// Short identifier used in the URL hash and in element ids.
    pub fn id(self) -> &'static str {
        match self {
            Layer::Political => "political",
            Layer::Monochrome => "mono",
            Layer::Imagery => "imagery",
            Layer::Terrain => "terrain",
            Layer::Population => "pop",
            Layer::GdpPerCapita => "gdp",
            Layer::Income => "income",
        }
    }

    /// Name shown to the user (status line and messages).
    pub fn label(self) -> &'static str {
        match self {
            Layer::Political => "Political",
            Layer::Monochrome => "Monochrome",
            Layer::Imagery => "Satellite",
            Layer::Terrain => "Terrain",
            Layer::Population => "Population",
            Layer::GdpPerCapita => "GDP per capita",
            Layer::Income => "Income group",
        }
    }

    pub fn from_id(id: &str) -> Option<Layer> {
        Layer::ALL.into_iter().find(|l| l.id() == id.trim())
    }

    /// Data file (its name in `assets/data/`) this layer needs before it can be
    /// shown, fetched the first time it is selected. Political and
    /// Monochrome are palettes over the base map and need none.
    pub fn data_file(self) -> Option<&'static str> {
        match self {
            Layer::Imagery => Some("blue_marble.webp"),
            Layer::Terrain => Some("terrain.webp"),
            _ => None,
        }
    }

    /// Whether the layer shows a whole-globe image (its `data_file`, a WebP)
    /// instead of a palette. Image layers share one texture, so only one of
    /// them is in GPU memory at a time.
    pub fn is_image(self) -> bool {
        matches!(self, Layer::Imagery | Layer::Terrain)
    }

    /// Value of the `u_layer` uniform in `globe.frag`.
    pub fn shader_index(self) -> i32 {
        match self {
            Layer::Political => 0,
            Layer::Monochrome => 1,
            // Image layers draw whatever image `u_imagery` holds.
            Layer::Imagery | Layer::Terrain => 2,
            // One id per theme, read as `u_layer >= 3` in `globe.frag`.
            Layer::Population => 3,
            Layer::GdpPerCapita => 4,
            Layer::Income => 5,
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
        self.data_file().is_some()
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

/// Projection named by `&proj=<id>` in a hash. Unknown or missing means the
/// globe, so old links keep working.
pub fn parse_hash_proj(hash: &str) -> Projection {
    hash.trim_start_matches('#')
        .split('&')
        .skip(1)
        .find_map(|kv| kv.strip_prefix("proj="))
        .and_then(Projection::from_id)
        .unwrap_or_default()
}

/// Latitude/longitude (radians) as e.g. `48.86° N  2.35° E`.
pub fn format_lat_lon(lat: f64, lon: f64) -> String {
    let (lat, lon) = (lat.to_degrees(), wrap_angle(lon).to_degrees());
    // Round first so a value that prints as 0.00 never gets an S or W.
    let round = |v: f64| (v * 100.0).round() / 100.0;
    let (lat, lon) = (round(lat), round(lon));
    format!(
        "{:.2}° {}  {:.2}° {}",
        lat.abs(),
        if lat >= 0.0 { 'N' } else { 'S' },
        lon.abs(),
        if lon >= 0.0 { 'E' } else { 'W' }
    )
}

/// Whole number with thousands separators, e.g. `1,234,567`.
pub fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Altitude in globe radii as a whole number of `units` with thousands
/// separators, e.g. `1,500 km`, `932 mi` or `810 nmi`.
pub fn format_altitude(alt: f64, units: Units) -> String {
    let v = (alt * EARTH_RADIUS_KM / units.km_per_unit()).round();
    // `max` also maps NaN to 0.
    format!("{} {}", group_thousands(v.max(0.0) as u64), units.suffix())
}

/// Distance unit for the scale bar, the measure panel and the altitude.
/// The choice is remembered in `localStorage` (not the URL hash: a shared
/// link should not impose the sharer's preference).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Units {
    Metric,
    Imperial,
    Nautical,
}

impl Units {
    /// The next unit in the cycle km -> mi -> nmi -> km.
    pub fn next(self) -> Self {
        match self {
            Units::Metric => Units::Imperial,
            Units::Imperial => Units::Nautical,
            Units::Nautical => Units::Metric,
        }
    }

    /// Kilometres in one main unit (km, mi or nmi).
    pub fn km_per_unit(self) -> f64 {
        match self {
            Units::Metric => 1.0,
            Units::Imperial => KM_PER_MILE,
            Units::Nautical => KM_PER_NMI,
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Units::Metric => "km",
            Units::Imperial => "mi",
            Units::Nautical => "nmi",
        }
    }

    /// Small unit used on the scale bar below one main unit, with its size
    /// in km. Metres for km, feet for miles. A nautical mile is exactly
    /// 1852 m and is defined from the metre, so sub-mile distances at sea
    /// are given in metres (cables, 0.1 nmi, are rarely used and would
    /// need a third unit and a fractional "0.5 cable" label).
    pub fn small(self) -> (&'static str, f64) {
        match self {
            Units::Metric | Units::Nautical => ("m", 0.001),
            Units::Imperial => ("ft", KM_PER_FOOT),
        }
    }

    /// Stable identifier stored in `localStorage`.
    pub fn id(self) -> &'static str {
        self.suffix()
    }

    /// Inverse of `id`; `None` for anything else (missing or corrupt value).
    pub fn from_id(id: &str) -> Option<Units> {
        [Units::Metric, Units::Imperial, Units::Nautical]
            .into_iter()
            .find(|u| u.id() == id)
    }
}

pub const KM_PER_MILE: f64 = 1.609_344;
pub const KM_PER_NMI: f64 = 1.852;
pub const KM_PER_FOOT: f64 = 0.000_304_8;
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
    let mut per_px = km_per_px / units.km_per_unit();
    let mut suffix = units.suffix();
    // Under one main unit, switch to the small one (m or ft) so the bar
    // reads "500 m" rather than "0.5 km".
    if max_px * per_px < 1.0 {
        let (small, km) = units.small();
        per_px = km_per_px / km;
        suffix = small;
    }
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
/// is appended as `&layer=<id>`, and a flat projection as `&proj=<id>` (the
/// globe writes nothing). On a flat map the center is the point on the
/// central meridian at that latitude, and there is never a bearing. When the terminator is on, `&sun=on` is
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
    if c.proj.is_flat() {
        hash.push_str("&proj=");
        hash.push_str(c.proj.id());
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
    let mut cam = Camera::new(
        lat.clamp(-90.0, 90.0),
        wrap_angle(lon.to_radians()).to_degrees(),
        (alt_km / EARTH_RADIUS_KM).clamp(MIN_ALT, MAX_ALT),
    )
    .with_bearing(bearing);
    cam.set_projection(parse_hash_proj(hash));
    Some(cam)
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

/// True for a mouse `pointermove` that Chromium synthesizes without the
/// mouse moving. After a layout change under a stationary pointer (or when a
/// page opens), it dispatches a "fake" move at the last known position, which
/// is (0, 0) if the mouse has never moved. Such an event arrives with no prior
/// cursor, no buttons held and zero movement, and must not count as hover. A
/// real first move only loses one event: the next one carries movement.
pub fn is_phantom_mouse_move(
    had_cursor: bool,
    buttons: u16,
    movement_x: f64,
    movement_y: f64,
) -> bool {
    !had_cursor && buttons == 0 && movement_x == 0.0 && movement_y == 0.0
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
    fn lat_lon_zero_has_no_sign() {
        assert_eq!(format_lat_lon(0.0, 0.0), "0.00° N  0.00° E");
        assert_eq!(format_lat_lon(-0.0, -0.0), "0.00° N  0.00° E");
        let tiny = (-0.001f64).to_radians();
        assert_eq!(format_lat_lon(tiny, tiny), "0.00° N  0.00° E");
        assert_eq!(
            format_lat_lon((-0.01f64).to_radians(), (-0.01f64).to_radians()),
            "0.01° S  0.01° W"
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
        assert_eq!(format_altitude(0.0, Units::Metric), "0 km");
        assert_eq!(
            format_altitude(400.0 / EARTH_RADIUS_KM, Units::Metric),
            "400 km"
        );
        assert_eq!(
            format_altitude(1500.0 / EARTH_RADIUS_KM, Units::Metric),
            "1,500 km"
        );
        assert_eq!(format_altitude(2.4, Units::Metric), "15,290 km");
        assert_eq!(
            format_altitude(1_234_567.0 / EARTH_RADIUS_KM, Units::Metric),
            "1,234,567 km"
        );
    }

    #[test]
    fn altitude_rounds_before_grouping() {
        // 15,999.7 km must not render as "15,1000 km".
        assert_eq!(
            format_altitude(15_999.7 / EARTH_RADIUS_KM, Units::Metric),
            "16,000 km"
        );
        assert_eq!(
            format_altitude(999.6 / EARTH_RADIUS_KM, Units::Metric),
            "1,000 km"
        );
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
    fn hash_carries_projection() {
        let mut c = Camera::new(46.8, 8.2, 3000.0 / EARTH_RADIUS_KM);
        // The globe keeps the old, shorter URL form.
        assert_eq!(
            format_view_hash(&c, Layer::Political, SunView::default()),
            "#46.8000,8.2000,3000"
        );
        c.set_projection(Projection::Mercator);
        let hash = format_view_hash(&c, Layer::Political, SunView::default());
        assert_eq!(hash, "#46.8000,8.2000,3000&proj=mercator");
        assert_eq!(parse_hash_proj(&hash), Projection::Mercator);
        let back = parse_view_hash(&hash).unwrap();
        assert_eq!(back.proj, Projection::Mercator);
        assert!(close(back.lat, c.lat, 1e-6) && close(back.lon, c.lon, 1e-6));
        // Together with the layer and the sun, in any order.
        let both = format_view_hash(
            &c,
            Layer::Monochrome,
            SunView {
                on: true,
                pin: Some(5.0),
            },
        );
        assert_eq!(
            both,
            "#46.8000,8.2000,3000&layer=mono&proj=mercator&sun=on&t=5"
        );
        assert_eq!(
            parse_hash_proj("#1,2,3&sun=on&proj=robinson&layer=mono"),
            Projection::Robinson
        );
        assert_eq!(parse_hash_layer(&both), Some(Layer::Monochrome));
        assert!(SunView::from_hash(&both).on);
        // Every projection round-trips through the hash.
        for p in Projection::ALL {
            c.set_projection(p);
            let h = format_view_hash(&c, Layer::Political, SunView::default());
            assert_eq!(parse_view_hash(&h).unwrap().proj, p, "{h}");
        }
    }

    #[test]
    fn old_links_and_bad_projections_mean_the_globe() {
        assert_eq!(parse_hash_proj("#46.8,8.2,600"), Projection::Globe);
        assert_eq!(parse_hash_proj(""), Projection::Globe);
        assert_eq!(parse_hash_proj("#1,2,3&proj=nope"), Projection::Globe);
        assert_eq!(parse_hash_proj("#1,2,3&proj="), Projection::Globe);
        // The view part never counts as a parameter.
        assert_eq!(parse_hash_proj("proj=mercator"), Projection::Globe);
        assert_eq!(
            parse_view_hash("#1,2,3&proj=nope").unwrap().proj,
            Projection::Globe
        );
    }

    #[test]
    fn flat_hash_has_no_bearing_and_clamps_mercator_latitude() {
        // A bearing in the link is dropped on a flat map.
        let c = parse_view_hash("#10,20,9000,135&proj=robinson").unwrap();
        assert_eq!(c.bearing, 0.0);
        assert!(c.north_up);
        // Mercator ends at about 85 degrees, so a link to the pole is pulled in.
        let c = parse_view_hash("#90,0,9000&proj=mercator").unwrap();
        assert!(close(c.lat.to_degrees(), 85.0511, 1e-3));
        let hash = format_view_hash(&c, Layer::Political, SunView::default());
        assert!(
            !hash.contains(",0.0&") && hash.ends_with("&proj=mercator"),
            "{hash}"
        );
        // Mollweide and the others keep the pole.
        let c = parse_view_hash("#90,0,9000&proj=mollweide").unwrap();
        assert!(close(c.lat.to_degrees(), 90.0, 1e-9));
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
    fn shader_indices_are_unique_except_image_layers() {
        // Image layers share index 2: the shader draws the image it is given.
        let mut seen: Vec<i32> = Layer::ALL
            .iter()
            .filter(|l| !l.is_image())
            .map(|l| l.shader_index())
            .collect();
        let n = seen.len();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), n);
        for l in Layer::ALL {
            assert_eq!(l.is_image(), l.shader_index() == 2, "{l:?}");
        }
    }

    #[test]
    fn thematic_layers_have_ids_themes_and_no_data_file() {
        let themed: Vec<Layer> = Layer::ALL
            .into_iter()
            .filter(|l| l.theme().is_some())
            .collect();
        assert_eq!(
            themed,
            [Layer::Population, Layer::GdpPerCapita, Layer::Income]
        );
        for l in themed {
            assert!(l.data_file().is_none(), "uses the country data");
            assert!(l.shader_index() >= 3, "globe.frag reads u_layer >= 3");
        }
        for l in [
            Layer::Political,
            Layer::Monochrome,
            Layer::Imagery,
            Layer::Terrain,
        ] {
            assert!(l.theme().is_none());
            assert!(l.shader_index() < 3);
        }
        let c = Camera::new(0.0, 0.0, 2.0);
        for (l, id) in [
            (Layer::Population, "pop"),
            (Layer::GdpPerCapita, "gdp"),
            (Layer::Income, "income"),
        ] {
            let hash = format_view_hash(&c, l, SunView::default());
            assert!(hash.ends_with(&format!("&layer={id}")), "{hash}");
            assert_eq!(parse_hash_layer(&hash), Some(l));
        }
        assert_eq!(parse_hash_layer("#1,2,3&layer=population"), None);
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
    fn scrubbed_times_round_trip_through_the_hash() {
        // Whatever the scrubber produces (slider, key step, play, the ends of
        // its range) survives format -> parse exactly, and "Now" (no pin)
        // writes no `t`.
        use crate::scrub;
        let base = 1_718_992_800.0;
        let times = [
            scrub::with_minute_of_day(base, 1_439),
            scrub::with_day_of_year(base, 366),
            scrub::shift(base, scrub::HOUR),
            scrub::shift(base, -scrub::DAY),
            scrub::year_days(base, 200),
            scrub::clamp_t(0.0),
            scrub::MAX_T,
        ];
        for t in times {
            let sun = SunView {
                on: true,
                pin: Some(t),
            };
            let hash = format_view_hash(&cam(), Layer::Political, sun);
            assert_eq!(SunView::from_hash(&hash), sun, "{hash}");
        }
        let live = SunView {
            on: true,
            pin: None,
        };
        assert!(!format_view_hash(&cam(), Layer::Political, live).contains("&t="));
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

    const ALL_UNITS: [Units; 3] = [Units::Metric, Units::Imperial, Units::Nautical];

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
            for units in ALL_UNITS {
                let bar = scale_bar(&c, units, 120.0).unwrap();
                assert!(bar.px <= 120.0 + 1e-9 && bar.px >= 120.0 / 2.5, "{bar:?}");
                // Bar length in km must equal px * km/px, and be a nice number.
                let value: f64 = bar.label.split(' ').next().unwrap().parse().unwrap();
                let small = bar.label.ends_with(" m") || bar.label.ends_with(" ft");
                let km = value
                    * if small {
                        units.small().1
                    } else {
                        units.km_per_unit()
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
        // 58 km is 31.3 nmi, so the round length is 20 nmi.
        let n = scale_bar(&c, Units::Nautical, 100.0).unwrap();
        assert_eq!(n.label, "20 nmi");
        assert!((n.px - 20.0 * 1.852 / c.km_per_pixel()).abs() < 1e-9);
    }

    #[test]
    fn scale_bar_small_units() {
        // alt 0.001: 0.005797 km/px, so 100 px is 580 m = 1,902 ft = 0.313 nmi.
        let c = sized(0.0, 0.001, 1200.0, 800.0);
        assert_eq!(scale_bar(&c, Units::Metric, 100.0).unwrap().label, "500 m");
        assert_eq!(
            scale_bar(&c, Units::Imperial, 100.0).unwrap().label,
            "1000 ft"
        );
        assert_eq!(
            scale_bar(&c, Units::Nautical, 100.0).unwrap().label,
            "500 m"
        );
        // The bar width matches the ground distance in the small unit.
        let m = scale_bar(&c, Units::Metric, 100.0).unwrap();
        assert!((m.px * c.km_per_pixel() - 0.5).abs() < 1e-9);
        let f = scale_bar(&c, Units::Imperial, 100.0).unwrap();
        assert!((f.px * c.km_per_pixel() - 1000.0 * KM_PER_FOOT).abs() < 1e-9);
    }

    #[test]
    fn scale_bar_switches_at_one_main_unit() {
        // Choose km/px so that 100 px is exactly `d` km via a target alt.
        let c = sized(0.0, 0.1, 1200.0, 800.0);
        let per_px = c.km_per_pixel();
        for units in ALL_UNITS {
            let one = units.km_per_unit();
            // Just over one main unit: main unit; just under: small unit.
            let over = scale_bar(&c, units, one * 1.01 / per_px).unwrap();
            assert!(over.label.ends_with(units.suffix()), "{over:?}");
            let under = scale_bar(&c, units, one * 0.99 / per_px).unwrap();
            assert!(under.label.ends_with(units.small().0), "{under:?}");
            assert!(!under.label.contains('.'), "{under:?}");
            assert!(under.px <= one * 0.99 / per_px + 1e-9);
        }
    }

    #[test]
    fn scale_bar_works_at_tiny_altitude() {
        // If MIN_ALT drops, the bar must stay finite and non-empty.
        for units in ALL_UNITS {
            for alt in [1e-4, 1e-6, 1e-9] {
                let bar = scale_bar(&sized(10.0, alt, 800.0, 600.0), units, 100.0).unwrap();
                assert!(bar.px.is_finite() && bar.px > 0.0 && bar.px <= 100.0 + 1e-9);
                let v: f64 = bar.label.split(' ').next().unwrap().parse().unwrap();
                assert!(v > 0.0, "{bar:?}");
            }
        }
    }

    #[test]
    fn units_ids_round_trip() {
        for u in ALL_UNITS {
            assert_eq!(Units::from_id(u.id()), Some(u));
        }
        for bad in ["", "KM", "miles", "null", " km"] {
            assert_eq!(Units::from_id(bad), None);
        }
    }

    #[test]
    fn altitude_follows_units() {
        let a = 1500.0 / EARTH_RADIUS_KM;
        assert_eq!(format_altitude(a, Units::Metric), "1,500 km");
        assert_eq!(format_altitude(a, Units::Imperial), "932 mi");
        assert_eq!(format_altitude(a, Units::Nautical), "810 nmi");
        assert_eq!(format_altitude(f64::NAN, Units::Nautical), "0 nmi");
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
    fn units_cycle() {
        assert_eq!(Units::Metric.next(), Units::Imperial);
        assert_eq!(Units::Imperial.next(), Units::Nautical);
        assert_eq!(Units::Nautical.next(), Units::Metric);
        assert_eq!(Units::Metric.next().next().next(), Units::Metric);
    }

    #[test]
    fn phantom_mouse_move_is_detected() {
        // Fake move at a stale position: no cursor yet, zero movement.
        assert!(is_phantom_mouse_move(false, 0, 0.0, 0.0));
        // A real move carries movement.
        assert!(!is_phantom_mouse_move(false, 0, 3.0, 0.0));
        assert!(!is_phantom_mouse_move(false, 0, 0.0, -1.0));
        // Once the cursor is known, a still move is harmless and kept.
        assert!(!is_phantom_mouse_move(true, 0, 0.0, 0.0));
        // A drag in progress is never phantom.
        assert!(!is_phantom_mouse_move(false, 1, 0.0, 0.0));
    }

    #[test]
    fn imagery_layer_round_trips_through_the_hash_and_share_link() {
        assert_eq!(Layer::from_id("imagery"), Some(Layer::Imagery));
        let c = Camera::new(46.8, 8.2, 600.0 / EARTH_RADIUS_KM);
        let hash = format_view_hash(&c, Layer::Imagery, SunView::default());
        assert_eq!(hash, "#46.8000,8.2000,600&layer=imagery");
        assert_eq!(parse_hash_layer(&hash), Some(Layer::Imagery));
        let url = share_url(
            "https://example.com/maps/",
            &c,
            Layer::Imagery,
            SunView::default(),
        );
        assert_eq!(
            url,
            "https://example.com/maps/#46.8000,8.2000,600&layer=imagery"
        );
        assert_eq!(Layer::Monochrome.next(), Layer::Imagery);
        assert_eq!(Layer::Imagery.next(), Layer::Terrain);
        assert_eq!(Layer::Terrain.next(), Layer::Population);
        assert_eq!(Layer::Income.next(), Layer::Political);
    }

    #[test]
    fn only_image_layers_need_data_and_their_files_ship() {
        for l in Layer::ALL {
            assert_eq!(l.data_file().is_some(), l.is_image(), "{l:?}");
            assert_eq!(
                crate::layer_loader::Loadable::needs_data(l),
                l.data_file().is_some()
            );
        }
        assert!(Layer::Imagery.is_image() && Layer::Terrain.is_image());
        for l in Layer::ALL.into_iter().filter(|l| l.is_image()) {
            let name = l.data_file().unwrap();
            assert!(name.ends_with(".webp"), "imagery::decode reads images");
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/data")
                .join(name);
            let len = std::fs::metadata(&path)
                .unwrap_or_else(|e| panic!("{name} is committed: {e}"))
                .len();
            assert!(len > 100_000 && len < 2_000_000, "{name}: {len} bytes");
        }
        assert_eq!(Layer::from_id("terrain"), Some(Layer::Terrain));
    }

    #[test]
    fn imagery_goes_through_the_loader_like_a_real_data_layer() {
        use crate::layer_loader::{DataState, Effect, LayerLoader};
        let mut l = LayerLoader::new(Layer::Political);
        // A URL naming the layer: nothing shows until the data arrives.
        assert_eq!(
            l.select(Layer::Imagery),
            vec![Effect::Fetch(Layer::Imagery)]
        );
        assert_eq!(l.shown(), Layer::Political);
        assert_eq!(l.pending(), Some(Layer::Imagery));
        // A failed fetch falls back, then retries on the next selection.
        let fail = l.finished(Layer::Imagery, Err("HTTP 404".into()));
        assert!(matches!(fail[0], Effect::Failed(Layer::Imagery, _)));
        assert_eq!(l.shown(), Layer::Political);
        assert_eq!(l.state(Layer::Imagery), DataState::Failed);
        assert_eq!(
            l.select(Layer::Imagery),
            vec![Effect::Fetch(Layer::Imagery)]
        );
        assert_eq!(
            l.finished(Layer::Imagery, Ok(())),
            vec![Effect::Show(Layer::Imagery)]
        );
        // Once loaded it switches at once and never fetches again.
        l.select(Layer::Monochrome);
        assert_eq!(l.select(Layer::Imagery), vec![Effect::Show(Layer::Imagery)]);
    }
}
