//! Place labels: which of the countries, cities, oceans and seas to show for
//! the current camera, and where. Browser-free; `label_layer.rs` draws the
//! result.
//!
//! The data is `assets/data/labels.tsv`, built by `scripts/convert_geojson.py`
//! (format described there). Selection runs on every frame the view changes
//! and is cheap by construction, in four steps:
//!
//! 1. **Altitude gate.** A label is a candidate only when the camera is below
//!    `max_altitude` for its kind and tier, so density grows with zoom. A
//!    country, ocean or sea also needs to be big enough on screen to hold its
//!    own name.
//! 2. **Horizon test.** `facing` is 0 on and beyond the horizon and ramps up
//!    to 1 a little way in, so labels never show on the far side and fade out
//!    near the limb instead of being squeezed against it. On a flat map the
//!    whole label box must lie inside the projected map (`box_in_map`), so
//!    nothing spills over the outline or the top and bottom edge.
//! 3. **Priority.** Candidates are ordered by `priority_class` (oceans, then
//!    countries, then cities by population tier), then labels that were
//!    already visible, then population.
//! 4. **Greedy collision.** Each candidate is placed unless its box overlaps a
//!    box already placed. At most `MAX_LABELS` are placed.
//!
//! Anchors are projected with `Camera::project`, which works on unit vectors,
//! so labels near the ±180° seam and at the poles need no special case.

use crate::camera::{spherical, Camera, TAN_HALF_FOV};

/// First line of `labels.tsv`.
pub const MAGIC: &str = "MAPSLABELS1";
/// Most labels placed at once.
pub const MAX_LABELS: usize = 90;
/// Empty space kept between two labels, in CSS pixels.
const PAD: f64 = 4.0;
/// Empty space kept between a label and a UI panel.
const KEEP_OUT_PAD: f64 = 4.0;
/// Radius of a city's dot and the gap between dot and text.
pub const DOT_R: f64 = 2.5;
pub const DOT_GAP: f64 = 4.0;
/// Fade durations in milliseconds.
const FADE_IN_MS: f64 = 160.0;
const FADE_OUT_MS: f64 = 110.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Country,
    City,
    Ocean,
}

impl Kind {
    /// Font size in CSS pixels.
    pub fn font_px(self) -> f64 {
        match self {
            Kind::Country => 11.0,
            Kind::City => 12.0,
            Kind::Ocean => 12.0,
        }
    }

    /// Height of the label box in CSS pixels.
    pub fn box_height(self) -> f64 {
        self.font_px() * 1.35
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub kind: Kind,
    /// Text as drawn (countries are upper case).
    pub text: String,
    /// Radians.
    pub lat: f64,
    pub lon: f64,
    /// 0 (most important) to 7.
    pub tier: u8,
    pub population: u64,
    /// Angular size in radians; 0 for cities.
    pub extent: f64,
    /// Text width in CSS pixels. Zero until the browser layer measures it.
    pub width: f64,
}

/// Parse `labels.tsv`. Malformed lines are skipped; a wrong header is an error.
pub fn parse(text: &str) -> Result<Vec<Label>, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(MAGIC) {
        return Err("not a labels file (bad header)".into());
    }
    let mut out = Vec::new();
    for line in lines {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 7 || f[1].is_empty() {
            continue;
        }
        let kind = match f[0] {
            "C" => Kind::Country,
            "P" => Kind::City,
            "O" => Kind::Ocean,
            _ => continue,
        };
        let (Ok(lat), Ok(lon), Ok(tier), Ok(pop), Ok(ext)) = (
            f[2].parse::<f64>(),
            f[3].parse::<f64>(),
            f[4].parse::<u8>(),
            f[5].parse::<u64>(),
            f[6].parse::<f64>(),
        ) else {
            continue;
        };
        if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0) {
            continue;
        }
        out.push(Label {
            kind,
            text: if kind == Kind::Country {
                f[1].to_uppercase()
            } else {
                f[1].to_string()
            },
            lat: lat.to_radians(),
            lon: lon.to_radians(),
            tier,
            population: pop,
            extent: if ext.is_finite() {
                ext.max(0.0).to_radians()
            } else {
                0.0
            },
            width: 0.0,
        });
    }
    Ok(out)
}

/// Highest camera altitude (globe radii) at which a label of this kind and
/// tier may show. This is what makes density grow with zoom.
pub fn max_altitude(kind: Kind, tier: u8) -> f64 {
    const CITY: [f64; 8] = [6.0, 3.0, 1.6, 0.9, 0.45, 0.25, 0.14, 0.1];
    const SEA: [f64; 4] = [6.0, 3.5, 1.4, 0.7];
    match kind {
        Kind::Country => crate::camera::MAX_ALT,
        Kind::City => CITY[usize::from(tier).min(CITY.len() - 1)],
        Kind::Ocean => SEA[usize::from(tier).min(SEA.len() - 1)],
    }
}

/// Collision order: lower wins. Oceans first, then countries, then seas and
/// cities interleaved by tier.
pub fn priority_class(kind: Kind, tier: u8) -> u8 {
    match kind {
        Kind::Ocean if tier == 0 => 0,
        Kind::Country => 1,
        Kind::Ocean => tier.saturating_add(1),
        Kind::City => tier.saturating_add(2),
    }
}

/// How visible a surface point is: 0 on or beyond the horizon, 1 well inside
/// it, ramping in between. Not finite input gives 0.
pub fn facing(cam: &Camera, lat: f64, lon: f64) -> f64 {
    if cam.is_flat() {
        // A flat map has no horizon: a point is visible if the map shows it.
        return if lat.is_finite() && lon.is_finite() && crate::projection::in_domain(cam.proj, lat)
        {
            1.0
        } else {
            0.0
        };
    }
    let a = spherical(lat, lon);
    let b = spherical(cam.lat, cam.lon);
    let cos_c = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let horizon = 1.0 / cam.dist();
    let ramp = 0.25 * (1.0 - horizon);
    let f = (cos_c - horizon) / ramp;
    if f.is_finite() {
        f.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    /// A rectangle from a DOM bounding box, or `None` if it is empty, not
    /// finite, or entirely outside the `width` x `height` viewport. Hidden
    /// elements report a zero-size box, so they drop out here.
    pub fn from_dom(x: f64, y: f64, w: f64, h: f64, width: f64, height: f64) -> Option<Rect> {
        if ![x, y, w, h].iter().all(|v| v.is_finite()) || w <= 0.0 || h <= 0.0 {
            return None;
        }
        let r = Rect {
            x0: x,
            y0: y,
            x1: x + w,
            y1: y + h,
        };
        (r.x1 > 0.0 && r.y1 > 0.0 && r.x0 < width && r.y0 < height).then_some(r)
    }

    fn overlaps(&self, o: &Rect, pad: f64) -> bool {
        self.x0 < o.x1 + pad && o.x0 < self.x1 + pad && self.y0 < o.y1 + pad && o.y0 < self.y1 + pad
    }
}

/// Where a label's text starts and how it is aligned, for an anchor at
/// `(x, y)`: cities put the text to the right of their dot, the rest center
/// it on the anchor.
pub fn text_x(kind: Kind, x: f64) -> f64 {
    match kind {
        Kind::City => x + DOT_R + DOT_GAP,
        _ => x,
    }
}

/// The screen box a label occupies when anchored at `(x, y)`.
pub fn label_rect(l: &Label, x: f64, y: f64) -> Rect {
    let h = l.kind.box_height() * 0.5;
    match l.kind {
        Kind::City => Rect {
            x0: x - DOT_R,
            y0: y - h,
            x1: x + DOT_R + DOT_GAP + l.width,
            y1: y + h,
        },
        _ => Rect {
            x0: x - l.width * 0.5,
            y0: y - h,
            x1: x + l.width * 0.5,
            y1: y + h,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// Index into the label list.
    pub index: usize,
    /// Anchor in CSS pixels.
    pub x: f64,
    pub y: f64,
    /// 0..1, from `facing`.
    pub facing: f64,
}

/// Pixels per radian of arc at the view center.
fn px_per_rad(cam: &Camera) -> f64 {
    if cam.is_flat() {
        return cam.flat_px();
    }
    0.5 * cam.width.min(cam.height).max(1.0) / (cam.alt * TAN_HALF_FOV)
}

/// Whether a screen box lies wholly on a flat map: its corners and edge
/// midpoints are all inside the projected map domain (the outline of
/// Robinson and Mollweide, the top and bottom edge of the cylindrical maps,
/// which repeat sideways and so have no left or right edge). A label that
/// fails this would spill over empty space.
pub fn box_in_map(cam: &Camera, r: &Rect) -> bool {
    let s = cam.flat_px();
    let (cx, cy) = (cam.width * 0.5, cam.height * 0.5);
    let ym = cam.flat_center_y();
    let xm = 0.5 * (r.x0 + r.x1);
    let ym_ = 0.5 * (r.y0 + r.y1);
    [r.x0, xm, r.x1].iter().all(|&sx| {
        [r.y0, ym_, r.y1].iter().all(|&sy| {
            crate::projection::inverse(cam.proj, (sx - cx) / s, ym - (sy - cy) / s).is_some()
        })
    })
}

/// (priority class, not shown before, population descending, index).
type SortKey = (u8, bool, std::cmp::Reverse<u64>, usize);

/// Choose and place labels for this camera. `was_shown` says whether a label
/// was visible last time, so labels that already show keep their spot against
/// equals as the view moves, which stops them flickering.
///
/// `blocked` lists screen rectangles (CSS pixels) that are covered by UI
/// panels. A label whose box would touch one is not a candidate, so a free
/// label can take its place.
pub fn select(
    cam: &Camera,
    labels: &[Label],
    blocked: &[Rect],
    was_shown: impl Fn(usize) -> bool,
) -> Vec<Placed> {
    if !(cam.alt.is_finite() && cam.alt > 0.0 && cam.width > 0.0 && cam.height > 0.0) {
        return Vec::new();
    }
    let ppr = px_per_rad(cam);
    // (class, not shown before, index) sorts the candidates; the label list
    // is already ordered by population within kind and tier.
    let mut cands: Vec<(SortKey, Placed, Rect)> = Vec::new();
    for (i, l) in labels.iter().enumerate() {
        if cam.alt > max_altitude(l.kind, l.tier) {
            continue;
        }
        // A country, ocean or sea must be big enough on screen for its name.
        if l.kind != Kind::City && l.extent * ppr < 0.6 * l.width {
            continue;
        }
        let f = facing(cam, l.lat, l.lon);
        if f <= 0.0 {
            continue;
        }
        let Some((x, y)) = cam.project(l.lat, l.lon) else {
            continue;
        };
        let r = label_rect(l, x, y);
        if r.x0 < 0.0 || r.y0 < 0.0 || r.x1 > cam.width || r.y1 > cam.height {
            continue;
        }
        if cam.is_flat() && !box_in_map(cam, &r) {
            continue;
        }
        if blocked.iter().any(|b| r.overlaps(b, KEEP_OUT_PAD)) {
            continue;
        }
        let key = (
            priority_class(l.kind, l.tier),
            !was_shown(i),
            std::cmp::Reverse(l.population),
            i,
        );
        cands.push((
            key,
            Placed {
                index: i,
                x,
                y,
                facing: f,
            },
            r,
        ));
    }
    cands.sort_by_key(|c| c.0);
    let mut placed: Vec<(Placed, Rect)> = Vec::new();
    for (_, p, r) in cands {
        if placed.len() >= MAX_LABELS {
            break;
        }
        if placed.iter().all(|(_, o)| !r.overlaps(o, PAD)) {
            placed.push((p, r));
        }
    }
    placed.into_iter().map(|(p, _)| p).collect()
}

/// Move a label's opacity toward its target. Immediate when the user prefers
/// reduced motion.
pub fn step_alpha(alpha: f64, target: f64, dt_ms: f64, reduced_motion: bool) -> f64 {
    if reduced_motion || !dt_ms.is_finite() {
        return target;
    }
    if target > alpha {
        (alpha + dt_ms / FADE_IN_MS).min(target)
    } else {
        (alpha - dt_ms / FADE_OUT_MS).max(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const DATA: &str = include_str!("../assets/data/labels.tsv");

    /// Real labels with a plausible text width (about 6.5 px per character).
    fn real_labels() -> Vec<Label> {
        let mut v = parse(DATA).expect("labels.tsv parses");
        for l in &mut v {
            l.width = l.text.chars().count() as f64 * 6.5;
        }
        v
    }

    fn cam(lat: f64, lon: f64, alt: f64) -> Camera {
        let mut c = Camera::new(lat, lon, alt);
        c.width = 1000.0;
        c.height = 700.0;
        c
    }

    fn city(name: &str, lat: f64, lon: f64, tier: u8, pop: u64) -> Label {
        Label {
            kind: Kind::City,
            text: name.into(),
            lat: lat.to_radians(),
            lon: lon.to_radians(),
            tier,
            population: pop,
            extent: 0.0,
            width: 60.0,
        }
    }

    fn none(_: usize) -> bool {
        false
    }

    #[test]
    fn parses_the_bundled_file() {
        let v = real_labels();
        assert!(v.len() > 3000);
        assert!(v
            .iter()
            .any(|l| l.kind == Kind::Country && l.text == "FRANCE"));
        assert!(v.iter().any(|l| l.kind == Kind::City && l.text == "Bern"));
        assert!(v
            .iter()
            .any(|l| l.kind == Kind::Ocean && l.text == "Black Sea"));
        for l in &v {
            assert!(
                l.lat.abs() <= std::f64::consts::FRAC_PI_2 && l.lon.abs() <= std::f64::consts::PI
            );
            assert!(!l.text.contains('\t'));
        }
    }

    #[test]
    fn parse_rejects_bad_header_and_skips_bad_lines() {
        assert!(parse("nope\nP\tA\t1\t2\t3\t4\t0\n").is_err());
        assert!(parse("").is_err());
        let text = format!(
            "{MAGIC}\nP\tOk\t10\t20\t1\t100\t0\nP\tBadLat\t99\t20\t1\t100\t0\nP\t\t1\t2\t1\t1\t0\nX\tKind\t1\t2\t1\t1\t0\nP\tShort\t1\t2\nP\tNaN\tNaN\t2\t1\t1\t0\n"
        );
        let v = parse(&text).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].text, "Ok");
    }

    #[test]
    fn facing_is_zero_beyond_the_horizon() {
        let c = cam(0.0, 0.0, 2.0);
        assert_eq!(facing(&c, 0.0, 0.0), 1.0);
        assert_eq!(facing(&c, 0.0, std::f64::consts::PI), 0.0);
        assert_eq!(facing(&c, 0.0, std::f64::consts::FRAC_PI_2), 0.0);
        // Just inside the horizon: visible but faded.
        let horizon = (1.0 / c.dist()).acos();
        let f = facing(&c, 0.0, horizon - 0.02);
        assert!(f > 0.0 && f < 1.0, "{f}");
        assert_eq!(facing(&c, 0.0, horizon + 0.01), 0.0);
        assert_eq!(facing(&cam(f64::NAN, 0.0, 2.0), 0.0, 0.0), 0.0);
    }

    #[test]
    fn never_selects_the_far_side() {
        let labels = real_labels();
        for (lat, lon, alt) in [
            (0.0, 0.0, 2.5),
            (47.0, 8.0, 0.5),
            (-30.0, 170.0, 6.0),
            (60.0, -100.0, 1.0),
        ] {
            let c = cam(lat, lon, alt);
            let picked = select(&c, &labels, &[], none);
            let horizon = (1.0 / c.dist()).acos();
            for p in picked {
                let l = &labels[p.index];
                let a = spherical(l.lat, l.lon);
                let b = spherical(c.lat, c.lon);
                let ang = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2])
                    .clamp(-1.0, 1.0)
                    .acos();
                assert!(
                    ang < horizon,
                    "{} at {ang} beyond horizon {horizon}",
                    l.text
                );
            }
        }
    }

    #[test]
    fn selected_labels_never_overlap_and_stay_on_screen() {
        let labels = real_labels();
        for (lat, lon, alt) in [
            (20.0, 0.0, 3.0),
            (47.0, 8.0, 0.4),
            (47.0, 8.0, 0.063),
            (40.0, -74.0, 0.2),
            (-5.0, 175.0, 1.5),
        ] {
            let c = cam(lat, lon, alt);
            let picked = select(&c, &labels, &[], none);
            let rects: Vec<Rect> = picked
                .iter()
                .map(|p| label_rect(&labels[p.index], p.x, p.y))
                .collect();
            assert!(picked.len() <= MAX_LABELS);
            for (i, a) in rects.iter().enumerate() {
                assert!(a.x0 >= 0.0 && a.y0 >= 0.0 && a.x1 <= c.width && a.y1 <= c.height);
                for b in &rects[i + 1..] {
                    assert!(!a.overlaps(b, 0.0), "overlap at {lat},{lon},{alt}");
                }
            }
        }
    }

    #[test]
    fn density_grows_with_zoom() {
        let labels = real_labels();
        let count = |alt: f64| select(&cam(47.0, 8.0, alt), &labels, &[], none).len();
        let (far, mid) = (count(4.0), count(0.6));
        assert!(far > 0);
        assert!(mid > far, "{far} {mid}");
        // City labels appear only once zoomed in far enough, and more of
        // them as it gets closer, until the view gets too small to hold many.
        let cities = |alt: f64| {
            select(&cam(47.0, 8.0, alt), &labels, &[], none)
                .iter()
                .filter(|p| labels[p.index].kind == Kind::City)
                .count()
        };
        assert!(cities(4.0) < cities(1.0));
        assert!(cities(1.0) < cities(0.3));
        assert!(cities(0.063) > 5);
    }

    #[test]
    fn the_whole_globe_shows_oceans_and_countries() {
        let labels = real_labels();
        let picked = select(&cam(20.0, 20.0, 3.0), &labels, &[], none);
        let has = |k: Kind| picked.iter().any(|p| labels[p.index].kind == k);
        assert!(has(Kind::Ocean) && has(Kind::Country));
    }

    #[test]
    fn bigger_place_wins_a_collision() {
        let big = city("Big", 10.0, 10.0, 2, 5_000_000);
        let small = city("Small", 10.01, 10.01, 2, 100_000);
        for order in [[0, 1], [1, 0]] {
            let pair = [big.clone(), small.clone()];
            let labels = [pair[order[0]].clone(), pair[order[1]].clone()];
            let picked = select(&cam(10.0, 10.0, 0.1), &labels, &[], none);
            assert_eq!(picked.len(), 1);
            assert_eq!(labels[picked[0].index].text, "Big");
        }
    }

    #[test]
    fn lower_tier_wins_over_population() {
        let a = city("Capital", 10.0, 10.0, 1, 1_000_000);
        let b = city("Bigger", 10.0, 10.0, 3, 9_000_000);
        let picked = select(&cam(10.0, 10.0, 0.1), &[b, a], &[], none);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].index, 1);
    }

    #[test]
    fn labels_already_shown_keep_their_spot() {
        let a = city("A", 10.0, 10.0, 3, 900_000);
        let b = city("B", 10.0, 10.0, 3, 1_000_000);
        let labels = [a, b];
        let fresh = select(&cam(10.0, 10.0, 0.1), &labels, &[], none);
        assert_eq!(fresh[0].index, 1);
        let sticky = select(&cam(10.0, 10.0, 0.1), &labels, &[], |i| i == 0);
        assert_eq!(sticky.len(), 1);
        assert_eq!(sticky[0].index, 0);
    }

    #[test]
    fn altitude_gate_hides_small_cities_when_far() {
        let c = city("Town", 0.0, 0.0, 6, 120_000);
        assert!(select(&cam(0.0, 0.0, 1.0), std::slice::from_ref(&c), &[], none).is_empty());
        assert_eq!(select(&cam(0.0, 0.0, 0.1), &[c], &[], none).len(), 1);
    }

    #[test]
    fn works_across_the_antimeridian() {
        let west = city("West", 0.0, 179.9, 0, 10_000_000);
        let east = city("East", 0.0, -179.9, 0, 9_000_000);
        for cam_lon in [179.95, -179.95, 180.0, -180.0] {
            let c = cam(0.0, cam_lon, 0.1);
            let picked = select(&c, &[west.clone(), east.clone()], &[], none);
            assert!(!picked.is_empty(), "camera lon {cam_lon}");
            for p in picked {
                // Both anchors are within about 0.1 degrees of the center.
                assert!(
                    (p.x - 500.0).abs() < 60.0 && (p.y - 350.0).abs() < 60.0,
                    "{p:?}"
                );
            }
        }
        // Seen from the east side both labels still collide into one.
        let c = cam(0.0, -179.95, 0.06);
        assert_eq!(select(&c, &[west, east], &[], none).len(), 1);
    }

    #[test]
    fn works_at_the_poles() {
        let pole = city("Pole", 90.0, 0.0, 0, 1_000_000);
        let south = city("South", -90.0, 123.0, 0, 1_000_000);
        let looking_down = cam(85.0, 40.0, 0.5);
        let picked = select(&looking_down, &[pole.clone(), south.clone()], &[], none);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].index, 0);
        let c = cam(-85.0, -170.0, 1.0);
        let picked = select(&c, &[pole, south], &[], none);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].index, 1);
    }

    #[test]
    fn degenerate_cameras_select_nothing() {
        let labels = real_labels();
        assert!(select(&cam(0.0, 0.0, f64::NAN), &labels, &[], none).is_empty());
        assert!(select(&cam(0.0, 0.0, f64::INFINITY), &labels, &[], none).is_empty());
        assert!(select(&cam(f64::NAN, 0.0, 1.0), &labels, &[], none).is_empty());
        let mut c = cam(0.0, 0.0, 1.0);
        c.width = 0.0;
        assert!(select(&c, &labels, &[], none).is_empty());
    }

    #[test]
    fn tiny_countries_are_hidden_until_they_fit() {
        let swiss = Label {
            kind: Kind::Country,
            text: "SWITZERLAND".into(),
            lat: 46.8f64.to_radians(),
            lon: 8.2f64.to_radians(),
            tier: 0,
            population: 8_000_000,
            extent: 3.5f64.to_radians(),
            width: 90.0,
        };
        assert!(select(
            &cam(46.8, 8.2, 3.0),
            std::slice::from_ref(&swiss),
            &[],
            none
        )
        .is_empty());
        assert_eq!(select(&cam(46.8, 8.2, 0.3), &[swiss], &[], none).len(), 1);
    }

    #[test]
    fn fade_moves_toward_the_target() {
        assert_eq!(step_alpha(0.0, 1.0, 16.0, true), 1.0);
        assert_eq!(step_alpha(1.0, 0.0, 16.0, true), 0.0);
        let a = step_alpha(0.0, 1.0, 16.0, false);
        assert!(a > 0.0 && a < 1.0);
        assert_eq!(step_alpha(0.95, 1.0, 100.0, false), 1.0);
        assert_eq!(step_alpha(0.05, 0.0, 100.0, false), 0.0);
        assert_eq!(step_alpha(0.5, 0.5, 16.0, false), 0.5);
        assert_eq!(step_alpha(0.2, 1.0, f64::NAN, false), 1.0);
    }

    #[test]
    fn priority_orders_oceans_countries_then_cities() {
        assert!(priority_class(Kind::Ocean, 0) < priority_class(Kind::Country, 0));
        assert!(priority_class(Kind::Country, 0) < priority_class(Kind::City, 0));
        assert!(priority_class(Kind::City, 0) < priority_class(Kind::City, 5));
    }

    #[test]
    fn no_duplicate_anchors_in_a_selection() {
        let labels = real_labels();
        let picked = select(&cam(47.0, 8.0, 0.2), &labels, &[], none);
        let set: HashSet<usize> = picked.iter().map(|p| p.index).collect();
        assert_eq!(set.len(), picked.len());
    }

    #[test]
    fn blocked_rect_hides_label_under_it() {
        let c = cam(10.0, 10.0, 0.1);
        let a = city("A", 10.0, 10.0, 0, 10);
        let (x, y) = c.project(a.lat, a.lon).unwrap();
        assert_eq!(select(&c, std::slice::from_ref(&a), &[], none).len(), 1);
        let under = Rect {
            x0: x - 5.0,
            y0: y - 5.0,
            x1: x + 5.0,
            y1: y + 5.0,
        };
        assert!(select(&c, std::slice::from_ref(&a), &[under], none).is_empty());
    }

    #[test]
    fn blocked_rect_lets_another_label_through() {
        // The big city loses its spot under a panel; a lower-ranked one that
        // would otherwise collide with it shows instead.
        let c = cam(10.0, 10.0, 0.1);
        let big = city("Big", 10.0, 10.0, 0, 1_000_000);
        let mut small = city("Small", 10.0, 10.0, 0, 10);
        small.lon += 0.0042; // about 40 px east, inside Big's box
        let both = [big.clone(), small];
        assert_eq!(select(&c, &both, &[], none).len(), 1);
        assert_eq!(select(&c, &both, &[], none)[0].index, 0);
        let (x, y) = c.project(big.lat, big.lon).unwrap();
        let panel = Rect {
            x0: x - 3.0,
            y0: y - 3.0,
            x1: x + 3.0,
            y1: y + 3.0,
        };
        let picked = select(&c, &both, &[panel], none);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].index, 1);
    }

    #[test]
    fn blocked_rect_keeps_a_margin() {
        let c = cam(10.0, 10.0, 0.1);
        let a = city("A", 10.0, 10.0, 0, 10);
        let (x, y) = c.project(a.lat, a.lon).unwrap();
        let r = label_rect(&a, x, y);
        // Touching the label's right edge, or within the margin: blocked.
        let near = Rect {
            x0: r.x1 + KEEP_OUT_PAD - 1.0,
            y0: r.y0,
            x1: r.x1 + 50.0,
            y1: r.y1,
        };
        assert!(select(&c, std::slice::from_ref(&a), &[near], none).is_empty());
        let far = Rect {
            x0: r.x1 + KEEP_OUT_PAD + 1.0,
            ..near
        };
        assert_eq!(select(&c, std::slice::from_ref(&a), &[far], none).len(), 1);
    }

    #[test]
    fn dom_rect_conversion_drops_hidden_and_bad_boxes() {
        let ok = Rect::from_dom(10.0, 20.0, 100.0, 50.0, 800.0, 600.0).unwrap();
        assert_eq!(
            ok,
            Rect {
                x0: 10.0,
                y0: 20.0,
                x1: 110.0,
                y1: 70.0
            }
        );
        // A hidden element reports a zero-size box.
        assert!(Rect::from_dom(0.0, 0.0, 0.0, 0.0, 800.0, 600.0).is_none());
        assert!(Rect::from_dom(0.0, 0.0, 10.0, 0.0, 800.0, 600.0).is_none());
        assert!(Rect::from_dom(f64::NAN, 0.0, 10.0, 10.0, 800.0, 600.0).is_none());
        assert!(Rect::from_dom(0.0, 0.0, f64::INFINITY, 10.0, 800.0, 600.0).is_none());
        // Fully off screen.
        assert!(Rect::from_dom(900.0, 0.0, 10.0, 10.0, 800.0, 600.0).is_none());
        assert!(Rect::from_dom(-50.0, 0.0, 10.0, 10.0, 800.0, 600.0).is_none());
    }

    #[test]
    fn flat_labels_are_placed_where_the_map_puts_them() {
        use crate::projection::Projection;
        for p in [
            Projection::Equirectangular,
            Projection::Mercator,
            Projection::Robinson,
            Projection::Mollweide,
        ] {
            let mut c = cam(20.0, 10.0, 2.0);
            c.set_projection(p);
            let list = real_labels();
            let placed = select(&c, &list, &[], |_| false);
            assert!(!placed.is_empty(), "{p:?} shows labels");
            for pl in &placed {
                let l = &list[pl.index];
                // The anchor is exactly the camera's projection of the place.
                let (x, y) = c.project(l.lat, l.lon).unwrap();
                assert_eq!((pl.x, pl.y), (x, y), "{p:?} {}", l.text);
                assert_eq!(pl.facing, 1.0, "no horizon on a flat map");
                // And picking that screen point gives the place back.
                let (la, lo) = c.pick(x, y).unwrap();
                assert!((la - l.lat).abs() < 1e-6, "{p:?} {}", l.text);
                assert!(
                    crate::camera::wrap_angle(lo - l.lon).abs() < 1e-6,
                    "{p:?} {}",
                    l.text
                );
            }
        }
    }

    #[test]
    fn flat_labels_on_the_far_side_of_the_globe_show() {
        use crate::projection::Projection;
        // Sydney is over the horizon from London on the globe, not on a map.
        let sydney = (-33.9f64.to_radians(), 151.2f64.to_radians());
        let mut c = cam(51.5, 0.0, 2.0);
        assert_eq!(facing(&c, sydney.0, sydney.1), 0.0);
        c.set_projection(Projection::Robinson);
        assert_eq!(facing(&c, sydney.0, sydney.1), 1.0);
        // Past Mercator's limit, and for bad input, nothing shows.
        c.set_projection(Projection::Mercator);
        assert_eq!(facing(&c, 88f64.to_radians(), 0.0), 0.0);
        assert_eq!(facing(&c, f64::NAN, 0.0), 0.0);
    }

    /// A cam with a roomy viewport so the whole world fits.
    fn flat_cam(p: crate::projection::Projection) -> Camera {
        let mut c = cam(0.0, 0.0, 3.0);
        c.set_projection(p);
        c
    }

    fn ocean(lat: f64, lon: f64, width: f64) -> Label {
        Label {
            kind: Kind::Ocean,
            text: "X".into(),
            lat: lat.to_radians(),
            lon: lon.to_radians(),
            tier: 0,
            population: 0,
            extent: 3.0,
            width,
        }
    }

    #[test]
    fn flat_label_straddling_the_outline_is_dropped() {
        use crate::projection::Projection;
        for p in [Projection::Mollweide, Projection::Robinson] {
            let c = flat_cam(p);
            // Anchor inside the map, 20 px from the left edge at lat 40,
            // with a 200 px wide label: it sticks out over space.
            let (lat, lon): (f64, f64) = (40.0, -150.0);
            let (x, _) = c.project(lat.to_radians(), lon.to_radians()).unwrap();
            let edge = c
                .project(lat.to_radians(), -std::f64::consts::PI + 1e-9)
                .unwrap()
                .0;
            assert!(x > edge, "anchor inside");
            let wide = ocean(lat, lon, 2.0 * (x - edge) + 80.0);
            assert!(select(&c, &[wide], &[], none).is_empty(), "{p:?} wide");
            // The same place with a short label fits and shows.
            let short = ocean(lat, lon, (x - edge).min(40.0));
            assert_eq!(select(&c, &[short], &[], none).len(), 1, "{p:?} short");
        }
    }

    #[test]
    fn mercator_label_over_the_top_or_bottom_edge_is_dropped() {
        use crate::projection::Projection;
        let c = flat_cam(Projection::Mercator);
        let top = c
            .project(crate::projection::MERCATOR_MAX_LAT - 1e-9, 0.0)
            .unwrap()
            .1;
        // Anchor 3 px below the top edge: the 16 px box pokes out above.
        let lat_of = |y: f64| c.pick(500.0, y).unwrap().0.to_degrees();
        let near = ocean(lat_of(top + 3.0), 0.0, 40.0);
        assert!(select(&c, &[near], &[], none).is_empty());
        let ok = ocean(lat_of(top + 30.0), 0.0, 40.0);
        assert_eq!(select(&c, &[ok], &[], none).len(), 1);
        // Sideways there is no edge: the map repeats (this box is wider than one world).
        let mut side = ocean(0.0, 0.0, 900.0);
        side.extent = 6.0;
        assert_eq!(select(&c, &[side], &[], none).len(), 1);
    }
}
