//! Planet positions from Keplerian elements. Browser-free.
//!
//! Uses the elements and rates of E. M. Standish and J. G. Williams,
//! "Approximate Positions of the Planets" (JPL Solar System Dynamics,
//! <https://ssd.jpl.nasa.gov/planets/approx_pos.html>): Table 1 for 1800 to
//! 2050 AD, and Tables 2a and 2b for 3000 BC to 3000 AD. Outside that span
//! there is no position. The elements give heliocentric orbits referred to
//! the mean ecliptic and equinox of J2000. Against JPL Horizons (the tests
//! pin the bounds) the direction is within 0.035 degrees for Mercury to
//! Mars and 0.2 degrees for Jupiter to Neptune; Standish and Williams quote
//! somewhat larger worst cases for the outer planets in the long span.
//! Plenty for drawing the solar system and the planets in the sky.
//!
//! Positions are in the J2000 frame. Converting to an observer's sky in
//! another epoch needs precession (about 50 arcseconds a year).
//!
//! Time is Terrestrial Time (TT), as Julian centuries from J2000.0. For a
//! UTC clock, `centuries_tt_from_unix` adds an estimate of Delta T.
//!
//! Earth is the Earth-Moon barycenter here: the Earth itself is within
//! about 4,700 km of it, which shifts a planet seen from Earth by under 3
//! arcseconds. The Moon needs its own theory.

// Used by the sky and solar system views (beads maps-4nv.8, maps-ztx.3);
// until then only the tests call it.
#![cfg_attr(not(test), allow(dead_code))]

use std::f64::consts::PI;

/// Julian date of J2000.0 (2000-01-01 12:00 TT).
pub const J2000_JD: f64 = 2_451_545.0;
/// Julian date of the Unix epoch (1970-01-01 00:00 UTC).
const UNIX_EPOCH_JD: f64 = 2_440_587.5;
/// Mean obliquity of the ecliptic at J2000.0, degrees (IAU 2006).
const OBLIQUITY_J2000_DEG: f64 = 23.439_279_44;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Planet {
    Mercury,
    Venus,
    /// The Earth-Moon barycenter.
    Earth,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
}

impl Planet {
    pub const ALL: [Planet; 8] = [
        Planet::Mercury,
        Planet::Venus,
        Planet::Earth,
        Planet::Mars,
        Planet::Jupiter,
        Planet::Saturn,
        Planet::Uranus,
        Planet::Neptune,
    ];

    fn index(self) -> usize {
        self as usize
    }
}

/// Elements at J2000 and their rates per Julian century: semi-major axis
/// (au), eccentricity, inclination (deg), mean longitude (deg), longitude of
/// perihelion (deg), longitude of the ascending node (deg).
type Elements = [[f64; 6]; 2];

/// Table 1: valid 1800 AD to 2050 AD.
#[rustfmt::skip]
const TABLE_1: [Elements; 8] = [
    // Mercury
    [[0.387_099_27, 0.205_635_93, 7.004_979_02, 252.250_323_50, 77.457_796_28, 48.330_765_93],
     [0.000_000_37, 0.000_019_06, -0.005_947_49, 149_472.674_111_75, 0.160_476_89, -0.125_340_81]],
    // Venus
    [[0.723_335_66, 0.006_776_72, 3.394_676_05, 181.979_099_50, 131.602_467_18, 76.679_842_55],
     [0.000_003_90, -0.000_041_07, -0.000_788_90, 58_517.815_387_29, 0.002_683_29, -0.277_694_18]],
    // EM Bary
    [[1.000_002_61, 0.016_711_23, -0.000_015_31, 100.464_571_66, 102.937_681_93, 0.0],
     [0.000_005_62, -0.000_043_92, -0.012_946_68, 35_999.372_449_81, 0.323_273_64, 0.0]],
    // Mars
    [[1.523_710_34, 0.093_394_10, 1.849_691_42, -4.553_432_05, -23.943_629_59, 49.559_538_91],
     [0.000_018_47, 0.000_078_82, -0.008_131_31, 19_140.302_684_99, 0.444_410_88, -0.292_573_43]],
    // Jupiter
    [[5.202_887_00, 0.048_386_24, 1.304_396_95, 34.396_440_51, 14.728_479_83, 100.473_909_09],
     [-0.000_116_07, -0.000_132_53, -0.001_837_14, 3_034.746_127_75, 0.212_526_68, 0.204_691_06]],
    // Saturn
    [[9.536_675_94, 0.053_861_79, 2.485_991_87, 49.954_244_23, 92.598_878_31, 113.662_424_48],
     [-0.001_250_60, -0.000_509_91, 0.001_936_09, 1_222.493_622_01, -0.418_972_16, -0.288_677_94]],
    // Uranus
    [[19.189_164_64, 0.047_257_44, 0.772_637_83, 313.238_104_51, 170.954_276_30, 74.016_925_03],
     [-0.001_961_76, -0.000_043_97, -0.002_429_39, 428.482_027_85, 0.408_052_81, 0.042_405_89]],
    // Neptune
    [[30.069_922_76, 0.008_590_48, 1.770_043_47, -55.120_029_69, 44.964_762_27, 131.784_225_74],
     [0.000_262_91, 0.000_051_05, 0.000_353_72, 218.459_453_25, -0.322_414_64, -0.005_086_64]],
];

/// Table 2a: valid 3000 BC to 3000 AD, with Table 2b for the outer planets.
#[rustfmt::skip]
const TABLE_2A: [Elements; 8] = [
    // Mercury
    [[0.387_098_43, 0.205_636_61, 7.005_594_32, 252.251_667_24, 77.457_718_95, 48.339_618_19],
     [0.000_000_00, 0.000_021_23, -0.005_901_58, 149_472.674_866_23, 0.159_400_13, -0.122_141_82]],
    // Venus
    [[0.723_321_02, 0.006_763_99, 3.397_775_45, 181.979_708_50, 131.767_557_13, 76.672_614_96],
     [-0.000_000_26, -0.000_051_07, 0.000_434_94, 58_517.815_602_60, 0.056_796_48, -0.272_741_74]],
    // EM Bary
    [[1.000_000_18, 0.016_731_63, -0.000_543_46, 100.466_915_72, 102.930_058_85, -5.112_603_89],
     [-0.000_000_03, -0.000_036_61, -0.013_371_78, 35_999.373_063_29, 0.317_952_60, -0.241_238_56]],
    // Mars
    [[1.523_712_43, 0.093_365_11, 1.851_818_69, -4.568_131_64, -23.917_447_84, 49.713_209_84],
     [0.000_000_97, 0.000_091_49, -0.007_247_57, 19_140.299_342_43, 0.452_236_25, -0.268_524_31]],
    // Jupiter
    [[5.202_480_19, 0.048_535_90, 1.298_614_16, 34.334_791_52, 14.274_952_44, 100.292_826_54],
     [-0.000_028_64, 0.000_180_26, -0.003_226_99, 3_034.903_717_57, 0.181_991_96, 0.130_246_19]],
    // Saturn
    [[9.541_498_83, 0.055_508_25, 2.494_241_02, 50.075_713_29, 92.861_360_63, 113.639_987_02],
     [-0.000_030_65, -0.000_320_44, 0.004_519_69, 1_222.114_947_24, 0.541_794_78, -0.250_150_02]],
    // Uranus
    [[19.187_979_48, 0.046_857_40, 0.772_981_27, 314.202_766_25, 172.434_044_41, 73.962_502_15],
     [-0.000_204_55, -0.000_015_50, -0.001_801_55, 428.495_125_95, 0.092_669_85, 0.057_396_99]],
    // Neptune
    [[30.069_527_52, 0.008_954_39, 1.770_055_20, 304.222_892_87, 46.681_587_24, 131.786_358_53],
     [0.000_064_47, 0.000_008_18, 0.000_224_00, 218.465_153_14, 0.010_099_38, -0.006_063_02]],
];

/// Table 2b: extra mean anomaly terms `b T² + c cos(f T) + s sin(f T)`
/// (degrees, with `f` in degrees per century) for Jupiter to Neptune.
#[rustfmt::skip]
const TABLE_2B: [[f64; 4]; 4] = [
    [-0.000_124_52, 0.060_640_60, -0.356_354_38, 38.351_250_00], // Jupiter
    [0.000_258_99, -0.134_344_69, 0.873_201_47, 38.351_250_00], // Saturn
    [0.000_583_31, -0.977_318_48, 0.176_892_45, 7.670_250_00], // Uranus
    [-0.000_413_48, 0.683_463_18, -0.101_625_47, 7.670_250_00], // Neptune
];

/// Julian centuries from J2000.0 of the Table 1 span, 1800-01-01 to
/// 2050-12-31 (inclusive).
const TABLE_1_SPAN: (f64, f64) = (-2.0, 0.51);
/// The Table 2 span, 3000 BC (astronomical year -2999) to 3000 AD.
const TABLE_2_SPAN: (f64, f64) = (-50.0, 10.0);

/// Julian centuries of TT from J2000.0 for a Julian date in TT.
pub fn centuries_from_jd(jd_tt: f64) -> f64 {
    (jd_tt - J2000_JD) / 36_525.0
}

/// Delta T = TT - UT in seconds, for a decimal year. The long-term parabola
/// of Morrison and Stephenson (2004), `-20 + 32 u²` with `u` in centuries
/// from 1820. Within about a minute of the observed value from 1700 to now,
/// and a few hours of uncertainty by 3000 BC, which is still small next to
/// the error of the elements for the slow outer planets.
pub fn delta_t_seconds(year: f64) -> f64 {
    let u = (year - 1820.0) / 100.0;
    -20.0 + 32.0 * u * u
}

/// Julian centuries of TT from J2000.0 for a UTC time in Unix seconds.
/// `None` for NaN or infinite times.
pub fn centuries_tt_from_unix(unix_secs: f64) -> Option<f64> {
    if !unix_secs.is_finite() {
        return None;
    }
    let jd_utc = unix_secs / 86_400.0 + UNIX_EPOCH_JD;
    let year = 2000.0 + (jd_utc - J2000_JD) / 365.25;
    Some(centuries_from_jd(jd_utc + delta_t_seconds(year) / 86_400.0))
}

/// Heliocentric position in au, in the J2000 ecliptic frame (x towards the
/// equinox, z towards the north ecliptic pole), at `t` Julian centuries of
/// TT from J2000.0. `None` outside 3000 BC to 3000 AD, or for NaN.
pub fn heliocentric_ecliptic(planet: Planet, t: f64) -> Option<[f64; 3]> {
    let in_span = |(lo, hi): (f64, f64)| t >= lo && t <= hi;
    let i = planet.index();
    let (el, extra) = if in_span(TABLE_1_SPAN) {
        (&TABLE_1[i], None)
    } else if in_span(TABLE_2_SPAN) {
        (&TABLE_2A[i], i.checked_sub(4).map(|j| TABLE_2B[j]))
    } else {
        return None; // also NaN
    };
    let [a, e, inc, l, peri, node] = std::array::from_fn(|k| el[0][k] + el[1][k] * t);
    let mut m = l - peri;
    if let Some([b, c, s, f]) = extra {
        let ft = (f * t).to_radians();
        m += b * t * t + c * ft.cos() + s * ft.sin();
    }
    let m = (m + 180.0).rem_euclid(360.0) - 180.0;
    let ecc_anomaly = solve_kepler(m.to_radians(), e);
    // Position in the orbital plane, x towards perihelion.
    let xp = a * (ecc_anomaly.cos() - e);
    let yp = a * (1.0 - e * e).sqrt() * ecc_anomaly.sin();
    let w = (peri - node).to_radians(); // argument of perihelion
    let (node, inc) = (node.to_radians(), inc.to_radians());
    let (sw, cw, sn, cn, si, ci) = (
        w.sin(),
        w.cos(),
        node.sin(),
        node.cos(),
        inc.sin(),
        inc.cos(),
    );
    Some([
        (cw * cn - sw * sn * ci) * xp + (-sw * cn - cw * sn * ci) * yp,
        (cw * sn + sw * cn * ci) * xp + (-sw * sn + cw * cn * ci) * yp,
        (sw * si) * xp + (cw * si) * yp,
    ])
}

/// Eccentric anomaly `E` with `E - e sin E = M` (radians), by Newton's
/// method from `E = M + e sin M`. Converges in a few steps for planetary
/// eccentricities (all below 0.21).
fn solve_kepler(m: f64, e: f64) -> f64 {
    let mut ea = m + e * m.sin();
    for _ in 0..10 {
        let d = (ea - e * ea.sin() - m) / (1.0 - e * ea.cos());
        ea -= d;
        if d.abs() < 1e-12 {
            break;
        }
    }
    ea
}

/// Rotate a J2000 ecliptic vector to the J2000 equator (x towards the
/// equinox, z towards the celestial north pole).
pub fn ecliptic_to_equatorial([x, y, z]: [f64; 3]) -> [f64; 3] {
    let (s, c) = OBLIQUITY_J2000_DEG.to_radians().sin_cos();
    [x, c * y - s * z, s * y + c * z]
}

/// Geometric position of `planet` seen from Earth, in au, J2000 equatorial
/// frame. Light time and aberration are not applied (under an arcminute).
/// `None` when `planet` is Earth or outside the elements' span.
pub fn geocentric_equatorial(planet: Planet, t: f64) -> Option<[f64; 3]> {
    if planet == Planet::Earth {
        return None;
    }
    let p = heliocentric_ecliptic(planet, t)?;
    let e = heliocentric_ecliptic(Planet::Earth, t)?;
    Some(ecliptic_to_equatorial([
        p[0] - e[0],
        p[1] - e[1],
        p[2] - e[2],
    ]))
}

/// Geometric position of the Sun seen from Earth, in au, J2000 equatorial.
pub fn sun_geocentric_equatorial(t: f64) -> Option<[f64; 3]> {
    let e = heliocentric_ecliptic(Planet::Earth, t)?;
    Some(ecliptic_to_equatorial([-e[0], -e[1], -e[2]]))
}

/// Right ascension in `[0, 2π)`, declination in `[-π/2, π/2]` (radians),
/// and distance, of an equatorial vector. `None` for the zero vector.
pub fn ra_dec(v: [f64; 3]) -> Option<(f64, f64, f64)> {
    let r = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if !r.is_finite() || r <= 0.0 {
        return None;
    }
    let ra = v[1].atan2(v[0]).rem_euclid(2.0 * PI);
    let dec = (v[2] / r).clamp(-1.0, 1.0).asin();
    Some((ra, dec, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// JPL Horizons (DE441) heliocentric positions of the system
    /// barycenters, J2000 ecliptic, au, at Julian dates in TDB. Query:
    /// EPHEM_TYPE=VECTORS, CENTER='500@10', REF_PLANE=ECLIPTIC,
    /// REF_SYSTEM=ICRF, COMMAND='1'...'8', fetched 2026-11-15.
    const HORIZONS: &[(Planet, [[f64; 4]; 8])] = include!("ephemeris_fixtures.in");

    fn angle_deg(a: [f64; 3], b: [f64; 3]) -> f64 {
        let dot: f64 = (0..3).map(|i| a[i] * b[i]).sum();
        let cross = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let c = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
        c.atan2(dot).to_degrees()
    }

    fn norm(v: [f64; 3]) -> f64 {
        (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
    }

    /// Worst heliocentric direction error (degrees) and relative distance
    /// error over the reference dates in `span`.
    fn worst(planet: Planet, span: (f64, f64)) -> (f64, f64) {
        let rows = HORIZONS.iter().find(|(p, _)| *p == planet).unwrap().1;
        let mut out = (0.0f64, 0.0f64);
        let mut n = 0;
        for [jd, x, y, z] in rows {
            let t = centuries_from_jd(jd);
            if t < span.0 || t > span.1 {
                continue;
            }
            let got = heliocentric_ecliptic(planet, t).unwrap();
            let want = [x, y, z];
            out.0 = out.0.max(angle_deg(got, want));
            out.1 = out.1.max((norm(got) / norm(want) - 1.0).abs());
            n += 1;
        }
        assert!(n >= 1, "{planet:?}: no reference dates in {span:?}");
        out
    }

    #[test]
    fn table_1_matches_horizons_1800_to_2050() {
        // Degrees and relative distance; a little above the measured worst.
        let bounds = [
            (Planet::Mercury, 0.01, 3e-5),
            (Planet::Venus, 0.008, 5e-5),
            (Planet::Earth, 0.004, 5e-5),
            (Planet::Mars, 0.03, 1e-4),
            (Planet::Jupiter, 0.15, 1.2e-3),
            (Planet::Saturn, 0.25, 2.5e-3),
            (Planet::Uranus, 0.05, 1e-3),
            (Planet::Neptune, 0.025, 5e-4),
        ];
        for (planet, deg, rel) in bounds {
            let (d, r) = worst(planet, TABLE_1_SPAN);
            assert!(d < deg && r < rel, "{planet:?}: {d:.4} deg, {r:.2e} rel");
        }
    }

    #[test]
    fn table_2_matches_horizons_3000_bc_to_3000_ad() {
        let bounds = [
            (Planet::Mercury, 0.015, 5e-5),
            (Planet::Venus, 0.04, 6e-5),
            (Planet::Earth, 0.012, 1.5e-4),
            (Planet::Mars, 0.06, 2.5e-4),
            (Planet::Jupiter, 0.3, 3e-3),
            (Planet::Saturn, 0.4, 5e-3),
            (Planet::Uranus, 0.2, 6e-3),
            (Planet::Neptune, 0.25, 2e-3),
        ];
        for (planet, deg, rel) in bounds {
            let (d, r) = worst(planet, (TABLE_2_SPAN.0, TABLE_1_SPAN.0));
            let (d2, r2) = worst(planet, (TABLE_1_SPAN.1, TABLE_2_SPAN.1));
            let (d, r) = (d.max(d2), r.max(r2));
            assert!(d < deg && r < rel, "{planet:?}: {d:.4} deg, {r:.2e} rel");
        }
    }

    #[test]
    fn no_position_outside_the_span_or_for_nan() {
        for t in [-50.01, 10.01, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for p in Planet::ALL {
                assert_eq!(heliocentric_ecliptic(p, t), None, "{p:?} at {t}");
            }
        }
        assert!(heliocentric_ecliptic(Planet::Mars, -50.0).is_some());
        assert!(heliocentric_ecliptic(Planet::Mars, 10.0).is_some());
    }

    #[test]
    fn tables_agree_where_they_meet() {
        // Just either side of 1800 and 2050, the two tables give nearly the
        // same direction, so a time scrubber does not see a jump.
        for edge in [TABLE_1_SPAN.0, TABLE_1_SPAN.1] {
            let (inside, outside) = if edge < 0.0 {
                (edge, edge - 1e-9)
            } else {
                (edge, edge + 1e-9)
            };
            for p in Planet::ALL {
                let a = heliocentric_ecliptic(p, inside).unwrap();
                let b = heliocentric_ecliptic(p, outside).unwrap();
                assert!(angle_deg(a, b) < 0.5, "{p:?} at {edge}");
            }
        }
    }

    #[test]
    fn kepler_solution_satisfies_the_equation() {
        for e in [0.0, 0.0068, 0.0934, 0.2056] {
            for k in -36..=36 {
                let m = (k as f64 * 5.0).to_radians();
                let ea = solve_kepler(m, e);
                assert!((ea - e * ea.sin() - m).abs() < 1e-12, "e {e} M {m}");
            }
        }
    }

    #[test]
    fn earth_sun_distance_and_seasons() {
        // J2000.0 is early January, near perihelion: about 0.983 au.
        let t0 = 0.0;
        let (_, _, r) = ra_dec(sun_geocentric_equatorial(t0).unwrap()).unwrap();
        assert!((r - 0.9833).abs() < 1e-3, "{r}");
        // Sun on the celestial equator near the 2000 March equinox
        // (2000-03-20 07:35 UTC), RA near 0.
        let t = centuries_from_jd(2_451_623.816);
        let (ra, dec, _) = ra_dec(sun_geocentric_equatorial(t).unwrap()).unwrap();
        assert!(dec.to_degrees().abs() < 0.02, "dec {}", dec.to_degrees());
        let ra = (ra + PI).rem_euclid(2.0 * PI) - PI;
        assert!(ra.to_degrees().abs() < 0.05, "ra {}", ra.to_degrees());
    }

    #[test]
    fn sun_agrees_with_the_day_night_module() {
        // `sun.rs` uses an independent formula; their declinations agree.
        // Near 2000 only: `sun.rs` gives coordinates of date, these are
        // J2000, and precession moves the declination by up to 0.006 deg a
        // year away from 2000.
        for unix in [930_000_000.0, 946_728_000.0, 960_000_000.0] {
            let t = centuries_tt_from_unix(unix).unwrap();
            let (_, dec, _) = ra_dec(sun_geocentric_equatorial(t).unwrap()).unwrap();
            let sub = crate::sun::subsolar_point(unix).unwrap();
            assert!((dec - sub.lat).abs().to_degrees() < 0.02, "{unix}");
        }
    }

    #[test]
    fn mars_from_earth_matches_known_opposition() {
        // Mars opposition 2003-08-28: closest in about 60,000 years,
        // 0.3727 au, near RA 22h 38m, Dec -15.8.
        let t = centuries_from_jd(2_452_879.5);
        let (ra, dec, r) = ra_dec(geocentric_equatorial(Planet::Mars, t).unwrap()).unwrap();
        assert!((r - 0.3727).abs() < 0.002, "{r}");
        assert!(
            (ra.to_degrees() / 15.0 - 22.63).abs() < 0.05,
            "ra {}",
            ra.to_degrees() / 15.0
        );
        assert!(
            (dec.to_degrees() + 15.8).abs() < 0.3,
            "dec {}",
            dec.to_degrees()
        );
    }

    #[test]
    fn earth_has_no_geocentric_position() {
        assert_eq!(geocentric_equatorial(Planet::Earth, 0.0), None);
    }

    #[test]
    fn ra_dec_edge_cases() {
        assert_eq!(ra_dec([0.0; 3]), None);
        assert_eq!(ra_dec([f64::NAN, 0.0, 0.0]), None);
        assert_eq!(ra_dec([f64::INFINITY, 0.0, 0.0]), None);
        let (ra, dec, r) = ra_dec([0.0, 0.0, 2.0]).unwrap();
        assert_eq!((ra, r), (0.0, 2.0));
        assert!((dec - PI / 2.0).abs() < 1e-15);
        let (ra, _, _) = ra_dec([0.0, -1.0, 0.0]).unwrap();
        assert!((ra - 1.5 * PI).abs() < 1e-15);
    }

    #[test]
    fn unix_time_conversion() {
        // J2000.0 is 2000-01-01 12:00 TT, 64.184 s before 12:00 UTC.
        let t = centuries_tt_from_unix(946_728_000.0 - 64.184).unwrap();
        assert!((t * 36_525.0 * 86_400.0).abs() < 60.0, "{t}");
        assert_eq!(centuries_tt_from_unix(f64::NAN), None);
        assert_eq!(centuries_tt_from_unix(f64::INFINITY), None);
        assert!((delta_t_seconds(1820.0) + 20.0).abs() < 1e-12);
    }
}
