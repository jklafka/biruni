//! Sun, Moon and planets for the sky view: where they are for an observer
//! at a time (from `ephemeris` and `moon`), how big and bright, and the
//! Moon's phase. Browser-free.
//!
//! Directions are unit vectors in the sky view's equatorial space (see
//! `sky.rs`: +Y the celestial pole, RA 0h on +Z, 6h on +X), J2000 like the
//! star catalog, so bodies and stars agree with each other.

use crate::ephemeris::{self, Planet};
use crate::moon;
use crate::sky::{local_sidereal_time, mul, Mat3, SkyCamera};
use crate::stars::{overlaps, Rect};

/// Kilometers in an astronomical unit.
const AU_KM: f64 = 149_597_870.7;
/// Mean radius of the Sun, km.
const SUN_RADIUS_KM: f64 = 696_000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Sun,
    Moon,
    Planet,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub name: &'static str,
    pub kind: Kind,
    /// Direction, sky equatorial space.
    pub dir: [f64; 3],
    /// Angular radius, radians.
    pub radius: f64,
    /// Typical visual magnitude (planets: a round mean value, see
    /// [`PLANETS`]; enough to size the dot and decide visibility).
    pub mag: f32,
    /// CSS color.
    pub color: &'static str,
    /// Illuminated fraction of the disk (the Moon; 1 for the others).
    pub lit: f64,
}

/// The planets shown, with a typical magnitude and a color. Magnitudes vary
/// with distance and phase (Venus -3.8 to -4.9, Mars -2.9 to +1.8); the
/// view uses these round values for size and visibility only.
pub const PLANETS: [(Planet, &str, f32, &str); 7] = [
    (Planet::Mercury, "Mercury", 0.0, "#d8c8b4"),
    (Planet::Venus, "Venus", -4.2, "#fff6dc"),
    (Planet::Mars, "Mars", 0.7, "#ff9b6b"),
    (Planet::Jupiter, "Jupiter", -2.3, "#f5e2bd"),
    (Planet::Saturn, "Saturn", 0.6, "#ecd59e"),
    (Planet::Uranus, "Uranus", 5.7, "#bfe6ea"),
    (Planet::Neptune, "Neptune", 7.8, "#a3baff"),
];

/// From `ephemeris`' frame (x towards the equinox, z towards the pole) to
/// the sky view's, normalized; also the length.
fn to_sky(v: [f64; 3]) -> Option<([f64; 3], f64)> {
    let r = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (r.is_finite() && r > 0.0).then(|| ([v[1] / r, v[2] / r, v[0] / r], r))
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The Sun, the Moon (seen from the observer, so with parallax) and the
/// planets at `unix_secs` for an observer at latitude `lat` and east
/// longitude `lon` (radians). Bodies outside the theories' time spans are
/// left out.
pub fn bodies(unix_secs: f64, lat: f64, lon: f64) -> Vec<Body> {
    let Some(t) = ephemeris::centuries_tt_from_unix(unix_secs) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let sun = ephemeris::sun_geocentric_equatorial(t);
    if let Some((dir, au)) = sun.and_then(to_sky) {
        out.push(Body {
            name: "Sun",
            kind: Kind::Sun,
            dir,
            radius: (SUN_RADIUS_KM / (au * AU_KM)).asin(),
            mag: -26.7,
            color: "#fff4d6",
            lit: 1.0,
        });
    }
    let lst = local_sidereal_time(unix_secs, lon);
    if let Some(m) = moon::topocentric_equatorial(t, lat, lst) {
        if let Some((dir, km)) = to_sky(m) {
            // Phase angle at the Moon, between the Sun and the observer.
            let lit = sun.map_or(1.0, |s| {
                let s_km = [s[0] * AU_KM, s[1] * AU_KM, s[2] * AU_KM];
                let to_sun = to_sky([s_km[0] - m[0], s_km[1] - m[1], s_km[2] - m[2]]);
                to_sun.map_or(1.0, |(d, _)| (1.0 - dot(d, dir)) / 2.0)
            });
            out.push(Body {
                name: "Moon",
                kind: Kind::Moon,
                dir,
                radius: (moon::MOON_RADIUS_KM / km).asin(),
                mag: -12.0,
                color: "#ece8de",
                lit,
            });
        }
    }
    for (planet, name, mag, color) in PLANETS {
        if let Some((dir, _)) = ephemeris::geocentric_equatorial(planet, t).and_then(to_sky) {
            out.push(Body {
                name,
                kind: Kind::Planet,
                dir,
                radius: 0.0,
                mag,
                color,
                lit: 1.0,
            });
        }
    }
    out
}

/// A body placed on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct OnScreen {
    pub name: &'static str,
    pub kind: Kind,
    /// Center and radius, CSS pixels.
    pub x: f64,
    pub y: f64,
    pub r: f64,
    pub color: &'static str,
    /// Illuminated fraction, and the screen direction towards the Sun
    /// (radians from +x towards +y, i.e. down the screen) for the Moon.
    pub lit: f64,
    pub sun_angle: f64,
}

/// Smallest drawn radius of the Sun and the Moon, CSS pixels, so they stay
/// visible in a wide view.
const MIN_DISK: f64 = 6.0;

/// The bodies to draw: above the horizon (the Sun and Moon while any of the
/// disk is), on screen, and for planets bright enough for `mag_limit`.
/// Planet dots grow with brightness like the stars.
pub fn on_screen(
    bodies: &[Body],
    cam: &SkyCamera,
    eq_to_hor: &Mat3,
    mag_limit: f32,
) -> Vec<OnScreen> {
    let px_per_rad = cam.half_short() / (2.0 * cam.tan_quarter());
    let sun = bodies.iter().find(|b| b.kind == Kind::Sun).map(|b| b.dir);
    let mut out = Vec::new();
    for b in bodies {
        let h = mul(eq_to_hor, b.dir);
        if h[1] < -b.radius.sin() || (b.kind == Kind::Planet && b.mag > mag_limit) {
            continue;
        }
        let Some((x, y)) = cam.project(h) else {
            continue;
        };
        let r = match b.kind {
            Kind::Planet => (1.6 + 0.35 * (mag_limit - b.mag) as f64).clamp(1.6, 4.5),
            _ => (b.radius * px_per_rad).max(MIN_DISK),
        };
        if x < -r || y < -r || x > cam.width + r || y > cam.height + r {
            continue;
        }
        // Towards the Sun on screen: project a point a little way from the
        // body along the great circle to the Sun.
        let sun_angle = match (b.kind, sun) {
            (Kind::Moon, Some(s)) => {
                let d = dot(s, b.dir);
                let toward = [0, 1, 2].map(|i| s[i] - d * b.dir[i]);
                let n = dot(toward, toward).sqrt();
                let step = 1e-3 / n.max(1e-12);
                let p = [0, 1, 2].map(|i| b.dir[i] + step * toward[i]);
                cam.project(mul(eq_to_hor, p))
                    .map_or(0.0, |(px, py)| (py - y).atan2(px - x))
            }
            _ => 0.0,
        };
        out.push(OnScreen {
            name: b.name,
            kind: b.kind,
            x,
            y,
            r,
            color: b.color,
            lit: b.lit,
            sun_angle,
        });
    }
    out
}

/// Where each body's name goes: the left end of the text (vertically
/// centered), to the right of the body, else below or above it, clear of
/// every body and of the names placed before it (earlier bodies win: the
/// Sun, the Moon, then the planets). `None` when no spot is free.
pub fn label_spots(shown: &[OnScreen]) -> Vec<Option<(f64, f64)>> {
    let disks: Vec<Rect> = shown
        .iter()
        .map(|b| (b.x - b.r, b.y - b.r, b.x + b.r, b.y + b.r))
        .collect();
    let mut placed: Vec<Rect> = Vec::new();
    shown
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let w = 7.0 * b.name.chars().count() as f64;
            let spots = [
                (b.x + b.r + 5.0, b.y),
                (b.x - w / 2.0, b.y + b.r + 11.0),
                (b.x - w / 2.0, b.y - b.r - 11.0),
            ];
            let free = spots.into_iter().find(|&(sx, sy)| {
                let r = (sx, sy - 8.0, sx + w, sy + 8.0);
                !placed.iter().any(|p| overlaps(&r, p))
                    && !disks
                        .iter()
                        .enumerate()
                        .any(|(j, d)| j != i && overlaps(&r, d))
            });
            if let Some((sx, sy)) = free {
                placed.push((sx, sy - 8.0, sx + w, sy + 8.0));
            }
            free
        })
        .collect()
}

/// How much daylight the sky has, from the Sun's altitude (radians): 0 in
/// the night (below -12 degrees, nautical twilight), 1 in full day (above
/// +6 degrees), smooth in between.
pub fn daylight(sun_alt: f64) -> f64 {
    let (lo, hi) = ((-12f64).to_radians(), 6f64.to_radians());
    let x = ((sun_alt - lo) / (hi - lo)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// How many magnitudes daylight takes off the faintest visible star: at
/// full day only the Moon, Venus and Jupiter at their best remain.
pub const DAYLIGHT_MAGNITUDES: f32 = 9.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::{eq_to_horizon, vec_horiz, vec_radec};

    /// 2024-01-25 17:54 UTC: full Moon. 2024-01-11 11:57 UTC: new Moon.
    const FULL: f64 = 1_706_205_240.0;
    const NEW: f64 = 1_704_974_220.0;

    fn find(b: &[Body], name: &str) -> Body {
        b.iter().find(|x| x.name == name).unwrap().clone()
    }

    #[test]
    fn everything_is_there_with_sensible_sizes() {
        let b = bodies(FULL, 0.7, -1.3);
        assert_eq!(b.len(), 9, "Sun, Moon, seven planets");
        let sun = find(&b, "Sun");
        let r = sun.radius.to_degrees();
        assert!(r > 0.26 && r < 0.275, "Sun's radius {r}");
        let moon = find(&b, "Moon");
        let r = moon.radius.to_degrees();
        assert!(r > 0.24 && r < 0.29, "Moon's radius {r}");
        for x in &b {
            let n = dot(x.dir, x.dir);
            assert!((n - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn the_sun_agrees_with_the_terminator() {
        // sun.rs gives the subsolar point; the Sun is overhead there.
        let t = 1_718_992_800.0; // 2024-06-21 18:00 UTC
        let s = crate::sun::subsolar_point(t).unwrap();
        let sun = find(&bodies(t, s.lat, s.lon), "Sun");
        let m = eq_to_horizon(s.lat, local_sidereal_time(t, s.lon));
        let (_, alt) = vec_horiz(mul(&m, sun.dir));
        assert!(alt.to_degrees() > 89.5, "{}", alt.to_degrees());
        let (_, dec) = vec_radec(sun.dir);
        assert!(
            (dec.to_degrees() - 23.44).abs() < 0.05,
            "solstice declination"
        );
    }

    #[test]
    fn the_moon_is_full_and_new_on_time_and_opposite_the_sun() {
        let full = bodies(FULL, 0.7, -1.3);
        let new = bodies(NEW, 0.7, -1.3);
        assert!(
            find(&full, "Moon").lit > 0.99,
            "{}",
            find(&full, "Moon").lit
        );
        assert!(find(&new, "Moon").lit < 0.01, "{}", find(&new, "Moon").lit);
        let e = dot(find(&full, "Moon").dir, find(&full, "Sun").dir);
        assert!(e < -0.99, "full Moon opposite the Sun");
        // A week after new: about half lit.
        let quarter = find(&bodies(NEW + 7.4 * 86_400.0, 0.7, -1.3), "Moon").lit;
        assert!((quarter - 0.5).abs() < 0.1, "{quarter}");
    }

    #[test]
    fn parallax_depends_on_the_observer() {
        let a = find(&bodies(FULL, 0.7, -1.3), "Moon").dir;
        let b = find(&bodies(FULL, -0.7, 1.8), "Moon").dir;
        let deg = dot(a, b).clamp(-1.0, 1.0).acos().to_degrees();
        assert!(deg > 0.2 && deg < 2.0, "{deg}");
        let sun_a = find(&bodies(FULL, 0.7, -1.3), "Sun").dir;
        let sun_b = find(&bodies(FULL, -0.7, 1.8), "Sun").dir;
        assert_eq!(sun_a, sun_b, "no parallax for the Sun");
    }

    #[test]
    fn no_bodies_outside_the_theories() {
        assert!(bodies(f64::NAN, 0.0, 0.0).is_empty());
        assert!(bodies(1e15, 0.0, 0.0).is_empty());
    }

    #[test]
    fn on_screen_places_lights_and_hides() {
        // Look straight at the full Moon from where it is local midnight
        // (17:54 UTC at 91.5 degrees east).
        let (lat, lon) = (0.4, 91.5f64.to_radians());
        let b = bodies(FULL, lat, lon);
        let moon = find(&b, "Moon");
        let m = eq_to_horizon(lat, local_sidereal_time(FULL, lon));
        let (az, alt) = vec_horiz(mul(&m, moon.dir));
        assert!(alt > 0.0, "the full Moon is up at this time and place");
        let cam = SkyCamera::new(az, alt, 60f64.to_radians());
        let s = on_screen(&b, &cam, &m, 6.0);
        let mo = s.iter().find(|o| o.name == "Moon").unwrap();
        assert!((mo.x - cam.width / 2.0).abs() < 1e-6 && (mo.y - cam.height / 2.0).abs() < 1e-6);
        assert_eq!(
            mo.r, MIN_DISK,
            "a 60-degree view shows the Moon at the minimum size"
        );
        assert!(
            s.iter().all(|o| o.name != "Sun"),
            "the Sun is down at full Moon"
        );
        // Zoomed to 1 degree the Moon fills half the shorter axis.
        let close = SkyCamera::new(az, alt, 1f64.to_radians());
        let r = on_screen(&b, &close, &m, 6.0)
            .into_iter()
            .find(|o| o.name == "Moon")
            .unwrap()
            .r;
        assert!((r / (close.height / 2.0) - 0.52).abs() < 0.06, "{r}");
        // The faint planets need a faint limit.
        let all = SkyCamera::new(0.0, 0.0, 150f64.to_radians());
        let bright = on_screen(&b, &all, &m, -1.0);
        assert!(bright
            .iter()
            .all(|o| o.name != "Neptune" && o.name != "Mars"));
    }

    #[test]
    fn the_lit_side_faces_the_sun() {
        // First quarter (a week after new): the Sun is about 90 degrees from
        // the Moon. Look between them (the horizon does not matter here:
        // compare the screen direction from the Moon to its lit side with
        // the direction to the Sun's place).
        let t = NEW + 7.4 * 86_400.0;
        let b = bodies(t, 0.0, 0.0);
        let (sun, moon) = (find(&b, "Sun").dir, find(&b, "Moon").dir);
        let mid = [0, 1, 2].map(|i| sun[i] + moon[i]);
        let n = dot(mid, mid).sqrt();
        let m = eq_to_horizon(0.0, local_sidereal_time(t, 0.0));
        let (az, alt) = vec_horiz(mul(&m, mid.map(|v| v / n)));
        let cam = SkyCamera::new(az, alt, 140f64.to_radians());
        let (sx, sy) = cam.project(mul(&m, sun)).unwrap();
        let (mx, my) = cam.project(mul(&m, moon)).unwrap();
        let want = (sy - my).atan2(sx - mx);
        // on_screen needs both above the horizon: lift the observer's
        // horizon out of the way by giving the bodies a huge radius.
        let lifted: Vec<Body> = b
            .iter()
            .map(|x| Body {
                radius: 1.6,
                ..x.clone()
            })
            .collect();
        let o = on_screen(&lifted, &cam, &m, 6.0);
        let got = o.iter().find(|x| x.name == "Moon").unwrap().sun_angle;
        let tau = std::f64::consts::TAU;
        let diff = (got - want + tau / 2.0).rem_euclid(tau) - tau / 2.0;
        // The great circle bends on screen, so allow some slack.
        assert!(diff.abs() < 0.35, "lit side {got} vs Sun {want}");
    }

    #[test]
    fn names_move_out_of_each_others_way() {
        let at = |name, x| OnScreen {
            name,
            kind: Kind::Planet,
            x,
            y: 100.0,
            r: 3.0,
            color: "#fff",
            lit: 1.0,
            sun_angle: 0.0,
        };
        // Venus's name to the right would run over the Sun: it goes below.
        let spots = label_spots(&[at("Sun", 120.0), at("Venus", 100.0)]);
        assert_eq!(spots[0], Some((128.0, 100.0)));
        let (_, y) = spots[1].unwrap();
        assert!(y > 100.0, "below");
        // Far apart: both to the right.
        let spots = label_spots(&[at("Sun", 100.0), at("Venus", 400.0)]);
        assert_eq!(spots[1], Some((408.0, 100.0)));
    }

    #[test]
    fn daylight_ramps_through_twilight() {
        assert_eq!(daylight((-30f64).to_radians()), 0.0);
        assert_eq!(daylight((-12f64).to_radians()), 0.0);
        assert_eq!(daylight(6f64.to_radians()), 1.0);
        let mid = daylight((-3f64).to_radians());
        assert!((mid - 0.5).abs() < 1e-9);
        assert!(daylight(0.0) > mid && daylight(0.0) < 1.0);
    }
}
