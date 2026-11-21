//! Spacecraft and small bodies for the solar system view, from
//! `assets/data/space.bin` (written by `scripts/fetch_space.py` from NASA
//! JPL's Horizons and Small-Body Database; the format is documented
//! there). Browser-free.
//!
//! Spacecraft are sampled positions over a window around the time the data
//! was fetched; between samples a cubic (Catmull-Rom) curve passes through
//! them, and outside the window a craft has no position. Small bodies are
//! osculating orbital elements, propagated as a two-body orbit about the
//! Sun for any time (elliptic, parabolic or hyperbolic): good near the
//! fetch date, drifting slowly away from it, and blind to close planetary
//! encounters.
//!
//! Positions come out in solar space (see `solar.rs`), au.

use crate::solar::ecl_to_solar;
use std::f64::consts::{PI, TAU};

/// The Sun's gravitational parameter, au^3 / day^2 (Gauss's constant
/// squared).
const GM_SUN: f64 = 0.000_295_912_208_285_591_1;
#[cfg(test)]
const DAY: f64 = 86_400.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Spacecraft,
    /// An asteroid or a dwarf planet.
    Asteroid,
    Comet,
}

/// Osculating elements: eccentricity, perihelion distance (au), angles in
/// radians, time of perihelion (Julian date, TDB).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Elements {
    pub epoch: f64,
    pub e: f64,
    pub q: f64,
    pub inc: f64,
    pub node: f64,
    pub peri: f64,
    pub tp: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Track {
    /// Sampled positions (J2000 ecliptic axes of `ephemeris`, au) from
    /// `start` (UTC Unix seconds), `step` seconds apart.
    Samples {
        start: f64,
        step: f64,
        points: Vec<[f64; 3]>,
    },
    Orbit(Elements),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub kind: Kind,
    pub name: String,
    pub track: Track,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Catalog {
    /// When the data was fetched (UTC Unix seconds).
    pub as_of: f64,
    pub objects: Vec<Object>,
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        let end = end.ok_or("space data: truncated")?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn f32(&mut self) -> Result<f64, String> {
        let b = self.take(4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64)
    }
    fn f64(&mut self) -> Result<f64, String> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(f64::from_le_bytes(a))
    }
}

/// Decode `space.bin`.
pub fn decode(data: &[u8]) -> Result<Catalog, String> {
    let mut r = Reader { data, pos: 0 };
    if r.take(4)? != b"SPCE" {
        return Err("space data: bad magic".into());
    }
    let version = r.u8()?;
    if version != 1 {
        return Err(format!("space data: unsupported version {version}"));
    }
    r.take(3)?;
    let as_of = r.f64()?;
    let count = r.u16()?;
    let mut objects = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let kind = match r.u8()? {
            0 => Kind::Spacecraft,
            1 => Kind::Asteroid,
            2 => Kind::Comet,
            k => return Err(format!("space data: unknown kind {k}")),
        };
        let len = r.u8()? as usize;
        let name = std::str::from_utf8(r.take(len)?)
            .map_err(|_| "space data: bad name")?
            .to_string();
        let track = if kind == Kind::Spacecraft {
            let start = r.f64()?;
            let step = r.f32()?;
            let n = r.u16()? as usize;
            let mut points = Vec::with_capacity(n);
            for _ in 0..n {
                points.push([r.f32()?, r.f32()?, r.f32()?]);
            }
            if !(start.is_finite() && step > 0.0 && n >= 2) {
                return Err(format!("space data: bad samples for {name}"));
            }
            Track::Samples {
                start,
                step,
                points,
            }
        } else {
            let v: Vec<f64> = (0..7).map(|_| r.f64()).collect::<Result<_, _>>()?;
            let el = Elements {
                epoch: v[0],
                e: v[1],
                q: v[2],
                inc: v[3].to_radians(),
                node: v[4].to_radians(),
                peri: v[5].to_radians(),
                tp: v[6],
            };
            if !(el.e >= 0.0 && el.q > 0.0 && v.iter().all(|x| x.is_finite())) {
                return Err(format!("space data: bad elements for {name}"));
            }
            Track::Orbit(el)
        };
        objects.push(Object { kind, name, track });
    }
    if r.pos != data.len() {
        return Err("space data: trailing bytes".into());
    }
    Ok(Catalog { as_of, objects })
}

/// Julian date (TDB, close enough to TT here) of a UTC Unix time.
fn jd_tt(unix: f64) -> Option<f64> {
    let t = crate::ephemeris::centuries_tt_from_unix(unix)?;
    Some(crate::ephemeris::J2000_JD + t * 36_525.0)
}

impl Elements {
    /// Rotate a point of the orbital plane (x towards perihelion) into the
    /// ecliptic frame of `ephemeris`.
    fn rotate(&self, xp: f64, yp: f64) -> [f64; 3] {
        let (sw, cw) = self.peri.sin_cos();
        let (sn, cn) = self.node.sin_cos();
        let (si, ci) = self.inc.sin_cos();
        [
            (cw * cn - sw * sn * ci) * xp + (-sw * cn - cw * sn * ci) * yp,
            (cw * sn + sw * cn * ci) * xp + (-sw * sn + cw * cn * ci) * yp,
            (sw * si) * xp + (cw * si) * yp,
        ]
    }

    /// Heliocentric position (`ephemeris` ecliptic axes, au) at true anomaly
    /// `nu`, or `None` past a hyperbola's asymptotes.
    pub fn at_true_anomaly(&self, nu: f64) -> Option<[f64; 3]> {
        let denom = 1.0 + self.e * nu.cos();
        if denom <= 1e-9 {
            return None;
        }
        let r = self.q * (1.0 + self.e) / denom;
        Some(self.rotate(r * nu.cos(), r * nu.sin()))
    }

    /// True anomaly at Julian date `jd` (TDB).
    pub fn true_anomaly(&self, jd: f64) -> Option<f64> {
        let (e, q) = (self.e, self.q);
        let dt = jd - self.tp;
        if !dt.is_finite() {
            return None;
        }
        let nu = if (e - 1.0).abs() < 1e-6 {
            // Parabola: Barker's equation, solved in closed form.
            let w = 3.0 * (GM_SUN / (2.0 * q * q * q)).sqrt() * dt;
            let y = (w / 2.0 + (w * w / 4.0 + 1.0).sqrt()).cbrt();
            2.0 * (y - 1.0 / y).atan()
        } else if e < 1.0 {
            let a = q / (1.0 - e);
            let n = (GM_SUN / (a * a * a)).sqrt();
            let m = (n * dt).rem_euclid(TAU);
            let mut ea = if e > 0.8 { PI } else { m };
            for _ in 0..50 {
                let d = (ea - e * ea.sin() - m) / (1.0 - e * ea.cos());
                ea -= d;
                if d.abs() < 1e-13 {
                    break;
                }
            }
            2.0 * ((1.0 + e).sqrt() * (ea / 2.0).sin()).atan2((1.0 - e).sqrt() * (ea / 2.0).cos())
        } else {
            let a = q / (e - 1.0);
            let n = (GM_SUN / (a * a * a)).sqrt();
            let m = n * dt;
            // e sinh H - H = M, by Newton from asinh(M / e).
            let mut h = (m / e).asinh();
            for _ in 0..60 {
                let d = (e * h.sinh() - h - m) / (e * h.cosh() - 1.0);
                h -= d;
                if d.abs() < 1e-13 * h.abs().max(1.0) {
                    break;
                }
            }
            2.0 * ((e + 1.0).sqrt() * (h / 2.0).sinh()).atan2((e - 1.0).sqrt() * (h / 2.0).cosh())
        };
        nu.is_finite().then_some(nu)
    }

    pub fn position(&self, jd: f64) -> Option<[f64; 3]> {
        self.at_true_anomaly(self.true_anomaly(jd)?)
    }
}

impl Object {
    /// Heliocentric position in solar space (au) at a UTC Unix time, or
    /// `None` outside a spacecraft's samples.
    pub fn position(&self, unix: f64) -> Option<[f64; 3]> {
        match &self.track {
            Track::Orbit(el) => el.position(jd_tt(unix)?).map(ecl_to_solar),
            Track::Samples {
                start,
                step,
                points,
            } => sample(*start, *step, points, unix).map(ecl_to_solar),
        }
    }

    /// The span of time (UTC Unix seconds) a spacecraft has positions for.
    pub fn span(&self) -> Option<(f64, f64)> {
        match &self.track {
            Track::Samples {
                start,
                step,
                points,
            } => Some((*start, *start + *step * (points.len() - 1) as f64)),
            Track::Orbit(_) => None,
        }
    }
}

/// Catmull-Rom interpolation through samples (end segments repeat their
/// end point as the missing neighbor).
fn sample(start: f64, step: f64, pts: &[[f64; 3]], unix: f64) -> Option<[f64; 3]> {
    let x = (unix - start) / step;
    let last = pts.len().checked_sub(1)?;
    if !(x >= 0.0 && x <= last as f64) {
        return None;
    }
    let i = (x.floor() as usize).min(last.saturating_sub(1));
    let f = x - i as f64;
    let p1 = pts[i];
    let p2 = pts[(i + 1).min(last)];
    let p0 = if i > 0 { pts[i - 1] } else { p1 };
    let p3 = if i + 2 <= last { pts[i + 2] } else { p2 };
    let (f2, f3) = (f * f, f * f * f);
    Some(std::array::from_fn(|k| {
        0.5 * (2.0 * p1[k]
            + (-p0[k] + p2[k]) * f
            + (2.0 * p0[k] - 5.0 * p1[k] + 4.0 * p2[k] - p3[k]) * f2
            + (-p0[k] + 3.0 * p1[k] - 3.0 * p2[k] + p3[k]) * f3)
    }))
}

/// A short form of a UTC date for the HUD: "as of 20 Nov 2026".
pub fn format_as_of(unix: f64) -> String {
    match crate::scrub::civil(unix) {
        Some(_) => format!("as of {}", crate::scrub::format_date(unix)),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Catalog {
        decode(include_bytes!("../scripts/testdata/tiny_space.bin")).unwrap()
    }

    fn close(a: [f64; 3], b: [f64; 3], eps: f64) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < eps)
    }

    fn by_name<'a>(c: &'a Catalog, name: &str) -> &'a Object {
        c.objects.iter().find(|o| o.name == name).unwrap()
    }

    #[test]
    fn the_fixture_decodes() {
        let c = fixture();
        assert_eq!(c.as_of, 1_779_235_200.0);
        assert_eq!(c.objects.len(), 10);
        assert_eq!(c.objects[0].name, "Circler");
        assert_eq!(by_name(&c, "Sondé").kind, Kind::Spacecraft, "UTF-8 names");
        assert_eq!(by_name(&c, "Ceres").kind, Kind::Asteroid);
        assert_eq!(by_name(&c, "3I/ATLAS").kind, Kind::Comet);
        // The committed snapshot decodes too.
        let snap = decode(include_bytes!("../assets/data/space.bin")).unwrap();
        assert!(snap.objects.len() >= 20);
        assert!(snap.objects.iter().any(|o| o.name == "Voyager 1"));
    }

    #[test]
    fn malformed_data_is_refused() {
        let good = include_bytes!("../scripts/testdata/tiny_space.bin");
        assert!(decode(&good[..good.len() - 1]).is_err());
        let mut more = good.to_vec();
        more.push(0);
        assert!(decode(&more).is_err());
        let mut bad = good.to_vec();
        bad[4] = 2;
        assert!(decode(&bad).is_err());
        assert!(decode(b"").is_err());
        assert!(decode(b"MAPD").is_err());
        let mut kind = good.to_vec();
        kind[18] = 9;
        assert!(decode(&kind).is_err());
    }

    #[test]
    fn samples_are_interpolated_and_bounded() {
        let c = fixture();
        let circ = by_name(&c, "Circler");
        let start = 1_767_225_600.0;
        // On the samples exactly (solar axes: x -> Z, y -> X).
        let p = circ.position(start + DAY).unwrap();
        assert!(close(
            p,
            ecl_to_solar([0.1f64.cos(), 0.1f64.sin(), 0.0]),
            1e-6
        ));
        // Between samples, a cubic through a circle stays on it.
        let mid = circ.position(start + 1.5 * DAY).unwrap();
        let r = (mid[0] * mid[0] + mid[1] * mid[1] + mid[2] * mid[2]).sqrt();
        assert!((r - 1.0).abs() < 1e-4, "{r}");
        // Outside the window: nothing.
        assert!(circ.position(start - 1.0).is_none());
        assert!(circ.position(start + 4.0 * DAY + 1.0).is_none());
        assert!(circ.position(start + 4.0 * DAY).is_some());
        assert!(circ.position(f64::NAN).is_none());
        assert_eq!(circ.span(), Some((start, start + 4.0 * DAY)));
    }

    #[test]
    fn a_circular_orbit_runs_at_the_right_speed() {
        // a = 1 au, e = 0: the period is 365.2569 days (Gauss), and the
        // body starts on the equinox direction at J2000.0.
        let c = fixture();
        let Track::Orbit(el) = by_name(&c, "Circle").track else {
            panic!()
        };
        let j2000 = crate::ephemeris::J2000_JD;
        assert!(close(el.position(j2000).unwrap(), [1.0, 0.0, 0.0], 1e-12));
        let year = TAU / GM_SUN.sqrt();
        assert!((year - 365.256_9).abs() < 1e-3, "{year}");
        assert!(close(
            el.position(j2000 + year / 4.0).unwrap(),
            [0.0, 1.0, 0.0],
            1e-9
        ));
        assert!(close(
            el.position(j2000 + year * 10.0).unwrap(),
            [1.0, 0.0, 0.0],
            1e-8
        ));
    }

    #[test]
    fn eccentric_parabolic_and_hyperbolic_orbits_obey_keplers_laws() {
        // Kepler's equation round trip for each kind: the time from
        // perihelion recomputed from the true anomaly matches.
        let mk = |e: f64, q: f64| Elements {
            epoch: 2_451_545.0,
            e,
            q,
            inc: 0.3,
            node: 1.0,
            peri: 2.0,
            tp: 2_451_545.0,
        };
        for (e, q) in [
            (0.2, 1.0),
            (0.967, 0.586),
            (0.999_9, 0.5),
            (1.0, 1.0),
            (1.000_000_1, 1.0),
            (6.14, 1.356),
        ] {
            let el = mk(e, q);
            for days in [-3000.0, -40.0, -1.0, 0.0, 0.5, 30.0, 700.0] {
                let nu = el.true_anomaly(el.tp + days).unwrap();
                let (back, period) = time_from_perihelion(e, q, nu);
                let mut diff = (back - days).abs();
                if let Some(p) = period {
                    let d = (back - days).rem_euclid(p);
                    diff = d.min(p - d);
                }
                let tol = 1e-7 * days.abs().max(1.0);
                assert!(diff < tol, "e {e}, {days} d: {back}");
                // Perihelion distance at perihelion.
                let p = el.position(el.tp).unwrap();
                let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                assert!((r - q).abs() < 1e-9, "e {e}: {r}");
            }
        }
    }

    /// Days from perihelion at true anomaly `nu` (the inverse, for tests),
    /// and the period of a closed orbit.
    fn time_from_perihelion(e: f64, q: f64, nu: f64) -> (f64, Option<f64>) {
        if (e - 1.0).abs() < 1e-6 {
            let d = (nu / 2.0).tan();
            return (
                (2.0 * q * q * q / GM_SUN).sqrt() * (d + d * d * d / 3.0),
                None,
            );
        }
        if e < 1.0 {
            let a = q / (1.0 - e);
            let ea = 2.0 * (((1.0 - e) / (1.0 + e)).sqrt() * (nu / 2.0).tan()).atan();
            let n = (GM_SUN / (a * a * a)).sqrt();
            return ((ea - e * ea.sin()) / n, Some(TAU / n));
        }
        let a = q / (e - 1.0);
        let h = 2.0 * (((e - 1.0) / (e + 1.0)).sqrt() * (nu / 2.0).tan()).atanh();
        ((e * h.sinh() - h) / (GM_SUN / (a * a * a)).sqrt(), None)
    }

    #[test]
    fn halley_comes_back_in_2061() {
        // 1P/Halley's next perihelion is 2061-07-28 (JD 2474052.3 in the
        // fixture's elements): about 0.586 au from the Sun then.
        let c = fixture();
        let halley = by_name(&c, "1P/Halley");
        let Track::Orbit(el) = halley.track else {
            panic!()
        };
        let p = el.position(2_474_052.32).unwrap();
        let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        assert!((r - 0.5725).abs() < 1e-3, "{r}");
        // Now (2026) it is far out, past Neptune's orbit... near aphelion at
        // about 35 au.
        let now = halley.position(1_795_000_000.0).unwrap();
        let r = (now[0] * now[0] + now[1] * now[1] + now[2] * now[2]).sqrt();
        assert!(r > 30.0 && r < 36.0, "{r}");
    }

    #[test]
    fn the_hyperbolic_visitor_leaves() {
        let c = fixture();
        let atlas = by_name(&c, "3I/ATLAS");
        let at = |unix: f64| {
            let p = atlas.position(unix).unwrap();
            (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()
        };
        // Perihelion 2025-10-29 at 1.36 au; years later, far away.
        assert!((at(1_761_696_000.0) - 1.3564).abs() < 0.01);
        assert!(at(1_761_696_000.0 + 5.0 * 365.25 * DAY) > 25.0);
    }

    #[test]
    fn as_of_text() {
        assert_eq!(format_as_of(1_795_132_800.0), "as of 20 Nov 2026");
    }
}
