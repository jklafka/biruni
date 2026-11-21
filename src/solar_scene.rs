//! One frame of the solar system view, browser-free: which bodies to draw
//! and where, the orbits as screen polylines, names and picking. Positions
//! come from `solar` in `f64`; what goes to the GPU is camera-relative (see
//! `solar.rs`). `solar_render.rs` draws the bodies, the overlay the orbits
//! and names.

use crate::ephemeris::{self, Planet};
use crate::sky::{mul, radec_vec, transpose};
use crate::solar::{ecl_to_solar, length, position, Body, Disk, SolarCamera, SOLAR_TO_EQ};
use crate::stars::{overlaps, Rect};

/// Below this radius (CSS pixels) a body is drawn as a dot of this size.
pub const DOT_RADIUS: f64 = 1.6;
/// Saturn's rings, inner edge of the C ring to the outer edge of the A ring,
/// in Saturn radii (60,268 km).
pub const SATURN_RINGS: (f64, f64) = (1.239, 2.270);

/// Display color, linear-ish RGB in 0..1.
pub fn color(body: Body) -> [f32; 3] {
    match body {
        Body::Sun => [1.0, 0.93, 0.78],
        Body::Mercury => [0.62, 0.58, 0.54],
        Body::Venus => [0.92, 0.85, 0.66],
        Body::Earth => [0.34, 0.52, 0.82],
        Body::Moon => [0.66, 0.65, 0.62],
        Body::Mars => [0.78, 0.40, 0.22],
        Body::Jupiter => [0.85, 0.72, 0.56],
        Body::Saturn => [0.89, 0.81, 0.60],
        Body::Uranus => [0.66, 0.86, 0.89],
        Body::Neptune => [0.38, 0.52, 0.88],
    }
}

/// CSS color for the orbit and name of a body.
pub fn css_color(body: Body) -> String {
    let [r, g, b] = color(body).map(|c| (c * 255.0).round() as u8);
    format!("rgb({r},{g},{b})")
}

/// North pole of rotation (IAU WGCCRE 2015, J2000 right ascension and
/// declination, degrees): for the bands of the giants and Saturn's rings.
pub fn pole(body: Body) -> (f64, f64) {
    match body {
        Body::Sun => (286.13, 63.87),
        Body::Mercury => (281.01, 61.42),
        Body::Venus => (272.76, 67.16),
        Body::Earth => (0.0, 90.0),
        Body::Moon => (266.86, 65.64),
        Body::Mars => (317.68, 52.89),
        Body::Jupiter => (268.06, 64.50),
        Body::Saturn => (40.59, 83.54),
        Body::Uranus => (257.31, -15.18),
        Body::Neptune => (299.36, 43.46),
    }
}

/// The pole as a unit vector in solar space.
pub fn pole_vec(body: Body) -> [f64; 3] {
    let (ra, dec) = pole(body);
    mul(
        &transpose(&SOLAR_TO_EQ),
        radec_vec(ra.to_radians(), dec.to_radians()),
    )
}

/// A body to draw this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawn {
    pub body: Body,
    /// Center relative to the camera, camera space, in units of the body's
    /// radius (so the sphere has radius 1 in the shader). See `solar.rs`
    /// on camera-relative coordinates.
    pub center: [f32; 3],
    /// Distance from the camera to the center, au.
    pub dist: f64,
    /// Outline on screen (CSS pixels): the sphere, and what must be covered
    /// (the rings, or the Sun's glow).
    pub disk: Disk,
    pub reach: Disk,
    /// Unit vector from the body towards the Sun, and the pole, camera space.
    pub sun: [f32; 3],
    pub pole: [f32; 3],
    /// Rings (inner and outer radius in body radii), if any.
    pub rings: Option<(f32, f32)>,
}

/// The bodies in front of the camera that touch the screen, farthest
/// first (the drawing order).
pub fn drawn(cam: &SolarCamera, eye: [f64; 3], t: f64) -> Vec<Drawn> {
    let basis_t = transpose(&cam.look.basis());
    let to_cam = |v: [f64; 3]| mul(&basis_t, v).map(|c| c as f32);
    let (w, h) = (cam.look.width, cam.look.height);
    let mut out: Vec<Drawn> = Body::ALL
        .iter()
        .filter_map(|&body| {
            let p = position(body, t)?;
            let r = body.radius_au();
            let disk = cam.disk(eye, p, r)?;
            let rings = (body == Body::Saturn).then_some(SATURN_RINGS);
            let reach = match (body, rings) {
                (_, Some((_, outer))) => cam.disk(eye, p, outer * r)?,
                (Body::Sun, _) => Disk {
                    r: disk.r * 1.6 + 40.0,
                    ..disk
                },
                _ => disk,
            };
            let pad = reach.r.max(DOT_RADIUS) + 2.0;
            if reach.x + pad < 0.0 || reach.y + pad < 0.0 || reach.x - pad > w || reach.y - pad > h
            {
                return None;
            }
            let rel = cam.relative(eye, p);
            let dist = length(rel);
            // A body behind the camera has no disk (`disk` refuses the
            // far side), so `rel` is ahead or to the side here.
            let sun = if body == Body::Sun {
                [0.0; 3]
            } else {
                let s = cam.relative(p, [0.0; 3]);
                let n = length(s);
                [s[0] / n, s[1] / n, s[2] / n].map(|c| c as f32)
            };
            Some(Drawn {
                body,
                center: rel.map(|c| (c / r) as f32),
                dist,
                disk,
                reach,
                sun,
                pole: to_cam(pole_vec(body)),
                rings: rings.map(|(a, b)| (a as f32, b as f32)),
            })
        })
        .collect();
    out.sort_by(|a, b| b.dist.total_cmp(&a.dist));
    out
}

/// Whether a point (au, solar space) is hidden behind one of the bodies.
fn hidden(eye: [f64; 3], p: [f64; 3], xy: (f64, f64), bodies: &[Occluder]) -> bool {
    let d = length([p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]]);
    bodies
        .iter()
        .any(|o| o.front < d && (xy.0 - o.disk.x).hypot(xy.1 - o.disk.y) < o.disk.r)
}

/// A body's outline and the distance to its near surface, for hiding the
/// parts of orbits behind it.
#[derive(Clone, Copy, Debug)]
pub struct Occluder {
    pub disk: Disk,
    pub front: f64,
}

pub fn occluders(drawn: &[Drawn]) -> Vec<Occluder> {
    drawn
        .iter()
        .filter(|d| d.disk.r > 1.0)
        .map(|d| Occluder {
            disk: d.disk,
            front: d.dist - d.body.radius_au(),
        })
        .collect()
}

/// Screen polylines (CSS pixels) of a curve `f(s)`, `s` from 0 to 1, split
/// where it goes behind a body or too far behind the view. Segments are
/// halved until they follow the curve to half a pixel on screen, and where
/// they cross a body's outline.
pub fn screen_path(
    cam: &SolarCamera,
    eye: [f64; 3],
    f: &dyn Fn(f64) -> [f64; 3],
    samples: usize,
    occluders: &[Occluder],
) -> Vec<Vec<(f64, f64)>> {
    #[derive(Clone, Copy)]
    struct Pt {
        s: f64,
        xy: Option<(f64, f64)>,
        hidden: bool,
    }
    let basis_t = transpose(&cam.look.basis());
    let (w, h) = (cam.look.width, cam.look.height);
    let at = |s: f64| -> Pt {
        let p = f(s);
        let rel = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]];
        let c = mul(&basis_t, rel);
        let n = length(c);
        // More than 100 degrees from the view center is far off screen and
        // close to the projection's pole: leave it out.
        let xy = if n > 0.0 && -c[2] / n > -0.17 {
            cam.look.project(rel)
        } else {
            None
        };
        let hidden = xy.is_none_or(|xy| hidden(eye, p, xy, occluders));
        Pt { s, xy, hidden }
    };
    let on_screen = |a: (f64, f64), b: (f64, f64)| {
        let m = 50.0;
        !(a.0.max(b.0) < -m || a.0.min(b.0) > w + m || a.1.max(b.1) < -m || a.1.min(b.1) > h + m)
    };
    let mut lines: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    let push = |p: &Pt, current: &mut Vec<(f64, f64)>, lines: &mut Vec<Vec<(f64, f64)>>| match (
        p.hidden, p.xy,
    ) {
        (false, Some(xy)) => current.push(xy),
        _ => {
            if current.len() > 1 {
                lines.push(std::mem::take(current));
            }
            current.clear();
        }
    };
    // Depth-first subdivision of each starting segment.
    let n = samples.max(2);
    let mut prev = at(0.0);
    push(&prev, &mut current, &mut lines);
    for i in 1..=n {
        let end = at(i as f64 / n as f64);
        let mut stack = vec![(end, 0u32)];
        while let Some((b, depth)) = stack.pop() {
            let a = prev;
            let split = match (a.xy, b.xy) {
                _ if depth >= 40 => false,
                (Some(pa), Some(pb)) if on_screen(pa, pb) => {
                    let m = at(0.5 * (a.s + b.s));
                    let len = (pa.0 - pb.0).hypot(pa.1 - pb.1);
                    match m.xy {
                        Some(pm) => {
                            let off =
                                (pm.0 - 0.5 * (pa.0 + pb.0)).hypot(pm.1 - 0.5 * (pa.1 + pb.1));
                            off > 0.5 || (a.hidden != b.hidden && len > 1.0) || m.hidden != a.hidden
                        }
                        None => true,
                    }
                }
                // One end projects and the other does not: find the edge.
                (Some(_), None) | (None, Some(_)) => depth < 24,
                _ => false,
            };
            if split {
                let m = at(0.5 * (a.s + b.s));
                stack.push((b, depth + 1));
                stack.push((m, depth + 1));
                continue;
            }
            push(&b, &mut current, &mut lines);
            prev = b;
        }
    }
    if current.len() > 1 {
        lines.push(current);
    }
    lines
}

/// Screen polylines, CSS pixels.
pub type Polylines = Vec<Vec<(f64, f64)>>;

/// An orbit to draw: whose, and its polylines.
pub struct OrbitLine {
    pub body: Body,
    pub lines: Vec<Vec<(f64, f64)>>,
}

/// The planets' orbits (from their current elements, through the planet)
/// and the Moon's path around the Earth over the last month.
pub fn orbits(cam: &SolarCamera, eye: [f64; 3], t: f64, occ: &[Occluder]) -> Vec<OrbitLine> {
    let mut out = Vec::new();
    for body in Body::ALL {
        let lines = match body {
            Body::Sun => continue,
            Body::Moon => {
                // A sidereal month back from now, about the Earth as it is
                // now, so the path closes on the Moon.
                let Some(earth) = position(Body::Earth, t) else {
                    continue;
                };
                let month = 27.321_661 / 36_525.0;
                let path = move |s: f64| {
                    let tt = t - s * month;
                    match (position(Body::Moon, tt), position(Body::Earth, tt)) {
                        (Some(m), Some(e)) => [
                            earth[0] + m[0] - e[0],
                            earth[1] + m[1] - e[1],
                            earth[2] + m[2] - e[2],
                        ],
                        _ => earth,
                    }
                };
                screen_path(cam, eye, &path, 96, occ)
            }
            _ => {
                let Some(planet) = body.planet() else {
                    continue;
                };
                let Some(o) = ephemeris::orbit(planet, t) else {
                    continue;
                };
                // The Earth's orbit is the barycenter's: shift it onto the
                // Earth itself, so it passes through the planet drawn.
                let shift = match (planet, position(body, t)) {
                    (Planet::Earth, Some(p)) => {
                        let b = ecl_to_solar(o.point(o.anomaly));
                        [p[0] - b[0], p[1] - b[1], p[2] - b[2]]
                    }
                    _ => [0.0; 3],
                };
                let a0 = o.anomaly;
                let path = move |s: f64| {
                    let p = ecl_to_solar(o.point(a0 + s * std::f64::consts::TAU));
                    [p[0] + shift[0], p[1] + shift[1], p[2] + shift[2]]
                };
                screen_path(cam, eye, &path, 128, occ)
            }
        };
        if !lines.is_empty() {
            out.push(OrbitLine { body, lines });
        }
    }
    out
}

/// A name on screen: the left end of the text, vertically centered.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub body: Body,
    pub x: f64,
    pub y: f64,
}

fn label_rect(x: f64, y: f64, name: &str) -> Rect {
    (x, y - 8.0, x + 7.0 * name.chars().count() as f64, y + 8.0)
}

/// Names to the right of their bodies, clear of each other: the target
/// first, then the bigger bodies on screen.
pub fn labels(cam: &SolarCamera, drawn: &[Drawn]) -> Vec<Label> {
    let mut order: Vec<&Drawn> = drawn.iter().collect();
    order.sort_by(|a, b| {
        (b.body == cam.target)
            .cmp(&(a.body == cam.target))
            .then(b.disk.r.total_cmp(&a.disk.r))
    });
    let mut placed: Vec<Rect> = Vec::new();
    let mut out = Vec::new();
    for d in order {
        let r = d.disk.r.max(DOT_RADIUS);
        let (x, y) = (d.disk.x + r + 5.0, d.disk.y);
        if !(0.0..cam.look.width).contains(&d.disk.x) || !(0.0..cam.look.height).contains(&y) {
            continue;
        }
        // A body bigger than the screen needs no name next to it.
        if d.disk.r > 0.5 * cam.look.width.min(cam.look.height) {
            continue;
        }
        let rect = label_rect(x, y, d.body.name());
        if placed.iter().any(|p| overlaps(&rect, p)) {
            continue;
        }
        placed.push(rect);
        out.push(Label { body: d.body, x, y });
    }
    out
}

/// Screen rectangles the bodies and their names take, for other names to
/// keep clear of.
pub fn taken(drawn: &[Drawn], labels: &[Label]) -> Vec<Rect> {
    let mut out: Vec<Rect> = drawn
        .iter()
        .map(|d| {
            let r = d.disk.r.max(DOT_RADIUS) + 2.0;
            (d.disk.x - r, d.disk.y - r, d.disk.x + r, d.disk.y + r)
        })
        .collect();
    out.extend(labels.iter().map(|l| label_rect(l.x, l.y, l.body.name())));
    out
}

/// The body under a screen point (CSS pixels): on its disk (with a few
/// pixels of slack for dots) or on its name. The nearest wins.
pub fn hit(drawn: &[Drawn], labels: &[Label], x: f64, y: f64) -> Option<Body> {
    let on_name = labels.iter().find(|l| {
        let (x0, y0, x1, y1) = label_rect(l.x, l.y, l.body.name());
        (x0..=x1).contains(&x) && (y0..=y1).contains(&y)
    });
    if let Some(l) = on_name {
        return Some(l.body);
    }
    drawn
        .iter()
        .rev() // nearest first
        .find(|d| (x - d.disk.x).hypot(y - d.disk.y) <= d.disk.r.max(8.0))
        .map(|d| d.body)
}

/// HUD text for a body: its name and the camera's distance from its center.
pub fn describe(body: Body, eye: [f64; 3], t: f64) -> Option<String> {
    let p = position(body, t)?;
    let d = length([p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]]);
    let above = (d - body.radius_au()).max(0.0);
    Some(format!(
        "{}, {} away",
        body.name(),
        crate::solar::format_distance(above)
    ))
}

/// Spacecraft, asteroids and comets (`space.rs`): markers, paths and names.
pub mod objects {
    use super::{screen_path, Occluder};
    use crate::solar::{ecl_to_solar, length, SolarCamera};
    use crate::space::{Catalog, Kind, Track};
    use crate::stars::{overlaps, Rect};

    /// Orbits and trajectories reach this far from the Sun at most, au.
    const MAX_R: f64 = 60.0;
    /// Markers closer than this (CSS pixels) to a click hit.
    pub const HIT_PX: f64 = 7.0;

    /// CSS color of each kind.
    pub fn css_color(kind: Kind) -> &'static str {
        match kind {
            Kind::Spacecraft => "#9fe3b0",
            Kind::Asteroid => "#c2b8a8",
            Kind::Comet => "#a6cdff",
        }
    }

    /// An object on screen.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Marker {
        /// Index into the catalog.
        pub index: usize,
        pub kind: Kind,
        pub x: f64,
        pub y: f64,
        /// Comets: unit vector on screen pointing away from the Sun.
        pub tail: Option<(f64, f64)>,
    }

    /// The objects in front of the camera, on screen and not behind a body,
    /// at UTC Unix time `unix`.
    pub fn markers(
        cat: &Catalog,
        cam: &SolarCamera,
        eye: [f64; 3],
        unix: f64,
        occ: &[Occluder],
    ) -> Vec<Marker> {
        let (w, h) = (cam.look.width, cam.look.height);
        let fwd = cam.look_dir();
        cat.objects
            .iter()
            .enumerate()
            .filter_map(|(index, o)| {
                let p = o.position(unix)?;
                let rel = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]];
                let d = length(rel);
                // Ahead of the camera, within 100 degrees of the view center.
                if !d.is_finite()
                    || d <= 0.0
                    || (rel[0] * fwd[0] + rel[1] * fwd[1] + rel[2] * fwd[2]) / d < -0.17
                {
                    return None;
                }
                let (x, y) = cam.look.project(rel)?;
                if !(-10.0..w + 10.0).contains(&x) || !(-10.0..h + 10.0).contains(&y) {
                    return None;
                }
                if occ
                    .iter()
                    .any(|b| b.front < d && (x - b.disk.x).hypot(y - b.disk.y) < b.disk.r)
                {
                    return None;
                }
                let tail = (o.kind == Kind::Comet)
                    .then(|| {
                        // A point a little farther from the Sun.
                        let r = length(p);
                        let k = 1.0 + 0.02 * (d / r).max(1e-6);
                        let q = p.map(|c| c * k);
                        let (tx, ty) =
                            cam.look
                                .project([q[0] - eye[0], q[1] - eye[1], q[2] - eye[2]])?;
                        let n = (tx - x).hypot(ty - y);
                        (n > 1e-9).then(|| ((tx - x) / n, (ty - y) / n))
                    })
                    .flatten();
                Some(Marker {
                    index,
                    kind: o.kind,
                    x,
                    y,
                    tail,
                })
            })
            .collect()
    }

    /// Paths: each spacecraft's trajectory over its whole data window, and
    /// each small body's orbit (an ellipse through the body, or the arms of
    /// a hyperbola or parabola out to 60 au). (catalog index, polylines).
    pub fn paths(
        cat: &Catalog,
        cam: &SolarCamera,
        eye: [f64; 3],
        unix: f64,
        occ: &[Occluder],
    ) -> Vec<(usize, super::Polylines)> {
        let mut out = Vec::new();
        for (i, o) in cat.objects.iter().enumerate() {
            let lines = match &o.track {
                Track::Samples { points, .. } => {
                    // Only while the spacecraft itself is shown.
                    let Some((t0, t1)) = o.span().filter(|&(a, b)| (a..=b).contains(&unix)) else {
                        continue;
                    };
                    let f = |s: f64| o.position(t0 + s * (t1 - t0)).unwrap_or([0.0; 3]);
                    screen_path(cam, eye, &f, points.len().clamp(2, 400), occ)
                }
                Track::Orbit(el) => {
                    if el.e < 1.0 {
                        // From the body's place now, once around.
                        let jd = crate::ephemeris::centuries_tt_from_unix(unix)
                            .map(|t| crate::ephemeris::J2000_JD + t * 36_525.0);
                        let nu0 = jd.and_then(|jd| el.true_anomaly(jd)).unwrap_or(0.0);
                        let aphelion = el.q * (1.0 + el.e) / (1.0 - el.e);
                        if aphelion > 4.0 * MAX_R {
                            continue;
                        }
                        let f = |s: f64| {
                            let nu = nu0 + s * std::f64::consts::TAU;
                            ecl_to_solar(el.at_true_anomaly(nu).unwrap_or([0.0; 3]))
                        };
                        screen_path(cam, eye, &f, 128, occ)
                    } else {
                        // Out to MAX_R on both arms: 1 + e cos nu = p / r.
                        let p = el.q * (1.0 + el.e);
                        let lim = ((p / MAX_R - 1.0) / el.e).clamp(-1.0, 1.0).acos();
                        let f = |s: f64| {
                            let nu = -lim + 2.0 * lim * s;
                            ecl_to_solar(el.at_true_anomaly(nu).unwrap_or([0.0; 3]))
                        };
                        screen_path(cam, eye, &f, 128, occ)
                    }
                }
            };
            if !lines.is_empty() {
                out.push((i, lines));
            }
        }
        out
    }

    /// Names for the markers, right of each, clear of `avoid` (the bodies
    /// and their names) and of each other: spacecraft first.
    pub fn labels(cat: &Catalog, markers: &[Marker], avoid: &[Rect]) -> Vec<(usize, f64, f64)> {
        let mut order: Vec<&Marker> = markers.iter().collect();
        order.sort_by_key(|m| match m.kind {
            Kind::Spacecraft => 0,
            Kind::Comet => 1,
            Kind::Asteroid => 2,
        });
        let mut placed = avoid.to_vec();
        let mut out = Vec::new();
        for m in order {
            let name = &cat.objects[m.index].name;
            let (x, y) = (m.x + 6.0, m.y);
            let rect = (x, y - 7.0, x + 6.0 * name.chars().count() as f64, y + 7.0);
            if placed.iter().any(|p| overlaps(&rect, p)) {
                continue;
            }
            placed.push(rect);
            out.push((m.index, x, y));
        }
        out
    }

    /// The marker nearest a screen point, within [`HIT_PX`].
    pub fn hit(markers: &[Marker], x: f64, y: f64) -> Option<usize> {
        markers
            .iter()
            .map(|m| ((m.x - x).hypot(m.y - y), m.index))
            .filter(|(d, _)| *d <= HIT_PX)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i)| i)
    }

    /// HUD text for an object: name, what it is, its distance from the Sun,
    /// and how fresh the data is.
    pub fn describe(cat: &Catalog, index: usize, unix: f64) -> Option<String> {
        let o = cat.objects.get(index)?;
        let r = length(o.position(unix)?);
        let what = match o.kind {
            Kind::Spacecraft => "spacecraft",
            Kind::Asteroid => "minor planet",
            Kind::Comet => "comet",
        };
        Some(format!(
            "{}, {what}, {} from the Sun (JPL, {})",
            o.name,
            crate::solar::format_distance(r),
            crate::space::format_as_of(cat.as_of)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ephemeris::centuries_tt_from_unix;
    use crate::solar::AU_KM;
    use std::f64::consts::PI;

    fn t0() -> f64 {
        centuries_tt_from_unix(1_735_689_600.0).unwrap()
    }

    fn sized(mut cam: SolarCamera) -> SolarCamera {
        cam.look.width = 1200.0;
        cam.look.height = 800.0;
        cam
    }

    #[test]
    fn the_home_view_shows_the_inner_planets() {
        let t = t0();
        let cam = sized(SolarCamera::default());
        let eye = cam.eye(t).unwrap();
        let d = drawn(&cam, eye, t);
        let names: Vec<Body> = d.iter().map(|d| d.body).collect();
        for b in [
            Body::Sun,
            Body::Mercury,
            Body::Venus,
            Body::Earth,
            Body::Mars,
        ] {
            assert!(names.contains(&b), "{b:?} missing from {names:?}");
        }
        // Farthest first.
        assert!(d.windows(2).all(|w| w[0].dist >= w[1].dist));
        // The Sun is in the middle; the planets are dots.
        let sun = d.iter().find(|d| d.body == Body::Sun).unwrap();
        assert!((sun.disk.x - 600.0).abs() < 1e-6 && (sun.disk.y - 400.0).abs() < 1e-6);
        for p in d.iter().filter(|d| d.body != Body::Sun) {
            assert!(p.disk.r < DOT_RADIUS, "{:?} {}", p.body, p.disk.r);
        }
    }

    #[test]
    fn centers_are_camera_relative_in_body_radii() {
        let t = t0();
        let mut cam = sized(SolarCamera::new(Body::Jupiter, 0.4, 0.1, 1.0));
        cam.set_dist(Body::Jupiter.arrival_dist());
        let eye = cam.eye(t).unwrap();
        let d = drawn(&cam, eye, t);
        let j = d.iter().find(|d| d.body == Body::Jupiter).unwrap();
        // Straight ahead (-Z), eight radii away.
        assert!(j.center[0].abs() < 1e-5 && j.center[1].abs() < 1e-5);
        assert!((j.center[2] + 8.0).abs() < 1e-5, "{:?}", j.center);
        // The Sun direction is a unit vector pointing back towards the Sun.
        let s = j.sun.map(|c| c as f64);
        assert!((length(s) - 1.0).abs() < 1e-6);
        let p = position(Body::Jupiter, t).unwrap();
        let want = mul(&transpose(&cam.look.basis()), p.map(|c| -c));
        let want = want.map(|c| c / length(p));
        assert!((0..3).all(|i| (s[i] - want[i]).abs() < 1e-6));
    }

    #[test]
    fn saturn_reaches_past_its_disk() {
        let t = t0();
        let mut cam = sized(SolarCamera::new(Body::Saturn, 0.0, -0.4, 1.0));
        cam.set_dist(Body::Saturn.arrival_dist());
        let eye = cam.eye(t).unwrap();
        let s = drawn(&cam, eye, t)
            .into_iter()
            .find(|d| d.body == Body::Saturn)
            .unwrap();
        assert!(s.rings.is_some());
        assert!(s.reach.r > 2.0 * s.disk.r);
        // The pole is tilted 26.7 degrees from the orbit's (and roughly the
        // ecliptic's) pole.
        let tilt = pole_vec(Body::Saturn)[1].acos().to_degrees();
        assert!((tilt - 28.0).abs() < 2.0, "{tilt}");
        let earth_tilt = pole_vec(Body::Earth)[1].acos().to_degrees();
        assert!((earth_tilt - 23.44).abs() < 0.01, "{earth_tilt}");
    }

    #[test]
    fn orbits_pass_through_their_planets() {
        let t = t0();
        let cam = sized(SolarCamera::new(Body::Sun, PI, 0.5, 4.0));
        let eye = cam.eye(t).unwrap();
        let d = drawn(&cam, eye, t);
        let occ = occluders(&d);
        let lines = orbits(&cam, eye, t, &occ);
        for body in [Body::Mercury, Body::Venus, Body::Earth, Body::Mars] {
            let o = lines.iter().find(|o| o.body == body).unwrap();
            let (px, py) = cam.project(eye, position(body, t).unwrap()).unwrap();
            let nearest = o
                .lines
                .iter()
                .flatten()
                .map(|(x, y)| (x - px).hypot(y - py))
                .fold(f64::INFINITY, f64::min);
            assert!(nearest < 1.0, "{body:?}: {nearest}");
        }
        // The Sun's disk hides the orbits behind it: no point of any line
        // is inside it unless in front of it (none are, the Sun is in the
        // middle of them all).
        let sun = d.iter().find(|d| d.body == Body::Sun).unwrap();
        for o in &lines {
            for (x, y) in o.lines.iter().flatten() {
                let r = (x - sun.disk.x).hypot(y - sun.disk.y);
                assert!(r >= sun.disk.r - 1.0, "{:?} at {r}", o.body);
            }
        }
    }

    #[test]
    fn a_close_orbit_is_cut_at_the_planet() {
        // From close to the Earth, its orbit runs through the planet: the
        // part behind it stops at the outline.
        let t = t0();
        let mut cam = sized(SolarCamera::new(Body::Earth, 1.0, 0.05, 1.0));
        cam.set_dist(Body::Earth.arrival_dist());
        let eye = cam.eye(t).unwrap();
        let d = drawn(&cam, eye, t);
        let earth = d.iter().find(|d| d.body == Body::Earth).unwrap().disk;
        let lines = orbits(&cam, eye, t, &occluders(&d));
        let o = lines.iter().find(|o| o.body == Body::Earth).unwrap();
        let ends: Vec<f64> = o
            .lines
            .iter()
            .flat_map(|l| [l[0], l[l.len() - 1]])
            .map(|(x, y)| (x - earth.x).hypot(y - earth.y))
            .collect();
        assert!(
            ends.iter().any(|r| (r - earth.r).abs() < 2.0),
            "{ends:?} vs {}",
            earth.r
        );
        for (x, y) in o.lines.iter().flatten() {
            assert!(x.is_finite() && y.is_finite());
        }
    }

    #[test]
    fn the_moon_path_closes_on_the_moon() {
        let t = t0();
        let mut cam = sized(SolarCamera::new(Body::Earth, 0.3, -1.2, 1.0));
        cam.set_dist(0.006);
        let eye = cam.eye(t).unwrap();
        let d = drawn(&cam, eye, t);
        let lines = orbits(&cam, eye, t, &occluders(&d));
        let o = lines.iter().find(|o| o.body == Body::Moon).unwrap();
        let (mx, my) = cam.project(eye, position(Body::Moon, t).unwrap()).unwrap();
        // It ends at the Moon's outline (the Moon hides its own center).
        let moon_r = d.iter().find(|d| d.body == Body::Moon).unwrap().disk.r;
        let nearest = o
            .lines
            .iter()
            .flatten()
            .map(|(x, y)| (x - mx).hypot(y - my))
            .fold(f64::INFINITY, f64::min);
        assert!(nearest < moon_r + 1.0, "{nearest} vs {moon_r}");
        // The loop is about the size of the Moon's orbit on screen.
        let (ex, ey) = cam.project(eye, position(Body::Earth, t).unwrap()).unwrap();
        let far = o
            .lines
            .iter()
            .flatten()
            .map(|(x, y)| (x - ex).hypot(y - ey))
            .fold(0.0, f64::max);
        let want = 400.0 * ((384_400.0 / AU_KM / 0.006f64).atan() / 2.0).tan()
            / (crate::solar::FOV / 4.0).tan();
        assert!(far > 0.6 * want && far < 1.6 * want, "{far} vs {want}");
    }

    #[test]
    fn labels_avoid_each_other_and_hits_find_bodies() {
        let t = t0();
        let cam = sized(SolarCamera::default());
        let eye = cam.eye(t).unwrap();
        let d = drawn(&cam, eye, t);
        let l = labels(&cam, &d);
        assert_eq!(l[0].body, Body::Sun, "the target first");
        for (i, a) in l.iter().enumerate() {
            for b in &l[i + 1..] {
                assert!(!overlaps(
                    &label_rect(a.x, a.y, a.body.name()),
                    &label_rect(b.x, b.y, b.body.name())
                ));
            }
        }
        // A click on a name or near a dot picks the body.
        let mars = l.iter().find(|l| l.body == Body::Mars).unwrap();
        assert_eq!(hit(&d, &l, mars.x + 3.0, mars.y), Some(Body::Mars));
        let m = d.iter().find(|d| d.body == Body::Mars).unwrap();
        assert_eq!(hit(&d, &[], m.disk.x + 4.0, m.disk.y), Some(Body::Mars));
        assert_eq!(hit(&d, &l, 5.0, 5.0), None);
        // HUD text.
        let s = describe(Body::Sun, eye, t).unwrap();
        assert!(
            s.starts_with("Sun, 3.50 au") || s.starts_with("Sun, 3.49 au"),
            "{s}"
        );
    }

    #[test]
    fn objects_are_placed_named_and_hit() {
        use crate::space::{decode, Kind};
        let cat = decode(include_bytes!("../assets/data/space.bin")).unwrap();
        // 2026-11-20, the snapshot's date, from far above the ecliptic.
        let unix = 1_795_132_800.0;
        let t = centuries_tt_from_unix(unix).unwrap();
        let cam = sized(SolarCamera::new(Body::Sun, PI, 1.2, 6.0));
        let eye = cam.eye(t).unwrap();
        let d = drawn(&cam, eye, t);
        let occ = occluders(&d);
        let m = objects::markers(&cat, &cam, eye, unix, &occ);
        let name = |i: usize| cat.objects[i].name.as_str();
        let names: Vec<&str> = m.iter().map(|m| name(m.index)).collect();
        for want in ["Ceres", "Vesta", "Parker Solar Probe", "JWST"] {
            assert!(names.contains(&want), "{want} not in {names:?}");
        }
        // Far ones are off this screen.
        assert!(!names.contains(&"Voyager 1"));
        // Comets carry a tail direction; others do not.
        for mk in &m {
            assert_eq!(
                mk.tail.is_some(),
                mk.kind == Kind::Comet,
                "{}",
                name(mk.index)
            );
        }
        // JWST sits next to the Earth (1.5 million km), within a few pixels.
        let jwst = m.iter().find(|mk| name(mk.index) == "JWST").unwrap();
        let (ex, ey) = cam.project(eye, position(Body::Earth, t).unwrap()).unwrap();
        assert!((jwst.x - ex).hypot(jwst.y - ey) < 5.0);
        // Names avoid what they are told to and each other.
        let avoid = vec![(ex - 50.0, ey - 50.0, ex + 50.0, ey + 50.0)];
        let l = objects::labels(&cat, &m, &avoid);
        assert!(!l.iter().any(|(i, _, _)| name(*i) == "JWST"));
        assert!(!l.is_empty());
        let ceres = m.iter().find(|mk| name(mk.index) == "Ceres").unwrap();
        assert_eq!(objects::hit(&m, ceres.x + 3.0, ceres.y), Some(ceres.index));
        let text = objects::describe(&cat, ceres.index, unix).unwrap();
        assert!(text.starts_with("Ceres, minor planet, 2."), "{text}");
        assert!(text.ends_with("(JPL, as of 20 Nov 2026)"), "{text}");
        // Paths: orbits for the small bodies, trajectories for the craft.
        let p = objects::paths(&cat, &cam, eye, unix, &occ);
        let has = |n: &str| p.iter().any(|(i, l)| name(*i) == n && !l.is_empty());
        assert!(has("Ceres") && has("Parker Solar Probe") && has("3I/ATLAS"));
    }

    #[test]
    fn outside_the_data_window_spacecraft_vanish() {
        use crate::space::decode;
        let cat = decode(include_bytes!("../assets/data/space.bin")).unwrap();
        let unix = 1_735_689_600.0; // 2025, before the snapshot's window
        let t = centuries_tt_from_unix(unix).unwrap();
        let cam = sized(SolarCamera::new(Body::Sun, PI, 1.2, 6.0));
        let eye = cam.eye(t).unwrap();
        let m = objects::markers(&cat, &cam, eye, unix, &[]);
        assert!(m.iter().all(|mk| mk.kind != crate::space::Kind::Spacecraft));
        assert!(m.iter().any(|mk| cat.objects[mk.index].name == "Ceres"));
        // Nor are their trajectories drawn.
        let p = objects::paths(&cat, &cam, eye, unix, &[]);
        assert!(p
            .iter()
            .all(|(i, _)| cat.objects[*i].kind != crate::space::Kind::Spacecraft));
    }

    #[test]
    fn paths_split_where_the_curve_leaves_the_view() {
        // A straight line through the camera's position: the part behind
        // the camera never shows.
        let cam = sized(SolarCamera::new(Body::Sun, 0.0, 0.0, 1.0));
        let eye = cam.eye(0.0).unwrap();
        let fwd = cam.look_dir();
        let path = move |s: f64| {
            let k = -2.0 + 4.0 * s;
            [
                eye[0] + fwd[0] * k + 0.01,
                eye[1] + fwd[1] * k,
                eye[2] + fwd[2] * k,
            ]
        };
        let lines = screen_path(&cam, eye, &path, 16, &[]);
        assert!(!lines.is_empty());
        for (x, y) in lines.iter().flatten() {
            assert!(x.is_finite() && y.is_finite());
        }
    }
}
