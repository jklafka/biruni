//! Sun position for the day/night terminator. Browser-free.
//!
//! Uses the low-precision solar position algorithm from the Astronomical
//! Almanac: mean longitude and anomaly give the ecliptic longitude, from
//! which declination and right ascension follow. The subsolar longitude is
//! the right ascension minus Greenwich mean sidereal time, so the equation
//! of time is included implicitly. Accurate to about 0.01 degrees for
//! 1950-2050, far better than the 1 degree needed for display.

/// Unix time of the J2000.0 epoch (2000-01-01 12:00 UTC), in seconds.
const J2000_UNIX: f64 = 946_728_000.0;

/// The point on Earth where the sun is directly overhead, in radians.
/// Longitude is in `(-pi, pi]`, east positive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Subsolar {
    pub lat: f64,
    pub lon: f64,
}

/// Subsolar point at a UTC time given as Unix seconds. `None` for NaN or
/// infinite times, and for times so far out the result would be meaningless.
pub fn subsolar_point(unix_secs: f64) -> Option<Subsolar> {
    if !unix_secs.is_finite() || unix_secs.abs() > 1e12 {
        return None;
    }
    let n = (unix_secs - J2000_UNIX) / 86_400.0; // days since J2000.0
    let l = (280.460 + 0.985_647_4 * n).rem_euclid(360.0); // mean longitude
    let g = (357.528 + 0.985_600_3 * n).rem_euclid(360.0).to_radians(); // mean anomaly
    let lambda = (l + 1.915 * g.sin() + 0.020 * (2.0 * g).sin()).to_radians();
    let eps = (23.439 - 0.000_000_4 * n).to_radians();
    let lat = (eps.sin() * lambda.sin()).asin();
    let ra = (eps.cos() * lambda.sin()).atan2(lambda.cos());
    let gmst = (280.460_618_37 + 360.985_647_366_29 * n)
        .rem_euclid(360.0)
        .to_radians();
    Some(Subsolar {
        lat,
        lon: wrap_pi(ra - gmst),
    })
}

/// Wrap an angle to `(-pi, pi]`.
fn wrap_pi(a: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    let w = (a + PI).rem_euclid(TAU) - PI;
    if w <= -PI {
        PI
    } else {
        w
    }
}

/// Unit vector from the globe center toward the sun, in world space
/// (+Y north pole, longitude 0 on +Z, 90 degrees east on +X).
pub fn sun_direction(unix_secs: f64) -> Option<[f32; 3]> {
    let s = subsolar_point(unix_secs)?;
    Some([
        (s.lat.cos() * s.lon.sin()) as f32,
        s.lat.sin() as f32,
        (s.lat.cos() * s.lon.cos()) as f32,
    ])
}

/// Day/night state requested by the URL hash.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SunHash {
    /// `&sun=on` or `&sun=off`; `None` when absent or unrecognised.
    pub enabled: Option<bool>,
    /// `&t=<unix seconds>` pins the sun to a fixed time (used by the visual
    /// tests so screenshots do not depend on the clock).
    pub time: Option<f64>,
}

/// Parse `&sun=` and `&t=` from a hash such as `#1,2,3&sun=on&t=1718992800`.
pub fn parse_hash_sun(hash: &str) -> SunHash {
    let mut out = SunHash::default();
    for kv in hash.trim_start_matches('#').split('&').skip(1) {
        if let Some(v) = kv.strip_prefix("sun=") {
            out.enabled = match v.trim() {
                "on" => Some(true),
                "off" => Some(false),
                _ => out.enabled,
            };
        } else if let Some(v) = kv.strip_prefix("t=") {
            if let Some(t) = v.trim().parse::<f64>().ok().filter(|t| t.is_finite()) {
                out.time = Some(t);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unix seconds for a UTC civil date and time (Howard Hinnant's algorithm).
    fn unix(y: i64, m: i64, d: i64, hh: i64, mm: i64) -> f64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        (days * 86_400 + hh * 3_600 + mm * 60) as f64
    }

    fn deg(r: f64) -> f64 {
        r.to_degrees()
    }

    #[test]
    fn unix_helper_matches_known_epochs() {
        assert_eq!(unix(1970, 1, 1, 0, 0), 0.0);
        assert_eq!(unix(2000, 1, 1, 12, 0), J2000_UNIX);
        assert_eq!(unix(2024, 6, 21, 18, 0), 1_718_992_800.0);
    }

    #[test]
    fn solstice_and_equinox_declinations() {
        // Reference times from the US Naval Observatory, 2024.
        let jun = subsolar_point(unix(2024, 6, 20, 20, 51)).unwrap();
        assert!((deg(jun.lat) - 23.44).abs() < 0.1, "{}", deg(jun.lat));
        let dec = subsolar_point(unix(2024, 12, 21, 9, 21)).unwrap();
        assert!((deg(dec.lat) + 23.44).abs() < 0.1, "{}", deg(dec.lat));
        let mar = subsolar_point(unix(2024, 3, 20, 3, 6)).unwrap();
        assert!(deg(mar.lat).abs() < 0.1, "{}", deg(mar.lat));
        let sep = subsolar_point(unix(2024, 9, 22, 12, 44)).unwrap();
        assert!(deg(sep.lat).abs() < 0.1, "{}", deg(sep.lat));
    }

    #[test]
    fn j2000_reference_values() {
        // 2000-01-01 12:00 UTC: declination -23.03 deg, equation of time
        // -3.3 min, so the sun is ~0.8 deg east of the prime meridian.
        let s = subsolar_point(J2000_UNIX).unwrap();
        assert!((deg(s.lat) + 23.03).abs() < 0.1, "{}", deg(s.lat));
        assert!((deg(s.lon) - 0.8).abs() < 0.2, "{}", deg(s.lon));
    }

    #[test]
    fn noon_longitude_follows_equation_of_time() {
        // Equation of time (apparent minus mean solar time) at 12:00 UTC;
        // each minute is 0.25 deg of longitude, positive EoT puts the sun west.
        for (y, m, d, eot_min) in [
            (2024, 2, 11, -14.2),
            (2024, 5, 14, 3.6),
            (2024, 7, 26, -6.5),
            (2024, 11, 3, 16.4),
        ] {
            let s = subsolar_point(unix(y, m, d, 12, 0)).unwrap();
            let want = -eot_min * 0.25;
            assert!(
                (deg(s.lon) - want).abs() < 0.3,
                "{y}-{m}-{d}: got {} want {want}",
                deg(s.lon)
            );
        }
    }

    #[test]
    fn subsolar_longitude_moves_west_15_degrees_per_hour() {
        let a = subsolar_point(unix(2024, 6, 21, 12, 0)).unwrap();
        let b = subsolar_point(unix(2024, 6, 21, 18, 0)).unwrap();
        assert!((deg(wrap_pi(a.lon - b.lon)) - 90.0).abs() < 0.2);
        // Midnight UTC puts the sun near the antimeridian, still in range.
        let c = subsolar_point(unix(2024, 6, 21, 0, 0)).unwrap();
        assert!(deg(c.lon).abs() > 170.0 && deg(c.lon).abs() <= 180.0);
    }

    #[test]
    fn year_boundaries_and_extreme_dates_are_continuous() {
        let before = subsolar_point(unix(2023, 12, 31, 23, 59)).unwrap();
        let after = subsolar_point(unix(2024, 1, 1, 0, 0)).unwrap();
        assert!((before.lat - after.lat).abs() < 1e-3);
        // One minute apart: the sun moves 0.25 deg west.
        assert!((deg(wrap_pi(before.lon - after.lon)) - 0.25).abs() < 0.02);
        // Dates well away from J2000 (1970, 2100) still give sane values.
        for t in [0.0, unix(2100, 12, 21, 12, 0), -1.0e9] {
            let s = subsolar_point(t).unwrap();
            assert!(deg(s.lat).abs() <= 23.5);
            assert!(s.lon > -std::f64::consts::PI && s.lon <= std::f64::consts::PI);
        }
    }

    #[test]
    fn non_finite_times_are_rejected() {
        for t in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300] {
            assert_eq!(subsolar_point(t), None);
            assert_eq!(sun_direction(t), None);
        }
    }

    #[test]
    fn sun_direction_is_unit_and_matches_world_convention() {
        for t in [
            0.0,
            J2000_UNIX,
            unix(2024, 6, 21, 18, 0),
            unix(2024, 12, 1, 3, 7),
        ] {
            let d = sun_direction(t).unwrap();
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-5);
        }
        // 18:00 UTC at the June solstice: sun over ~90 W (-X), tilted north.
        let d = sun_direction(unix(2024, 6, 21, 18, 0)).unwrap();
        assert!(d[0] < -0.85 && d[1] > 0.38 && d[2].abs() < 0.1, "{d:?}");
        // 12:00 UTC at the equinox: sun over lon ~0 (+Z), on the equator.
        let d = sun_direction(unix(2024, 3, 20, 12, 0)).unwrap();
        assert!(d[2] > 0.99 && d[1].abs() < 0.15, "{d:?}");
    }

    #[test]
    fn hash_parsing() {
        assert_eq!(parse_hash_sun(""), SunHash::default());
        assert_eq!(parse_hash_sun("#1,2,3"), SunHash::default());
        let h = parse_hash_sun("#1,2,3&sun=on&t=1718992800");
        assert_eq!(h.enabled, Some(true));
        assert_eq!(h.time, Some(1_718_992_800.0));
        assert_eq!(
            parse_hash_sun("#1,2&layer=mono&sun=off").enabled,
            Some(false)
        );
        // Malformed values are ignored.
        assert_eq!(parse_hash_sun("#1,2&sun=maybe&t=abc"), SunHash::default());
        assert_eq!(parse_hash_sun("#1,2&t=NaN").time, None);
        assert_eq!(parse_hash_sun("#1,2&t=inf").time, None);
        // The first segment is the view, never a parameter.
        assert_eq!(parse_hash_sun("#sun=on"), SunHash::default());
    }
}
