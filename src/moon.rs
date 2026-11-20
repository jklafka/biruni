//! The Moon's position: the main periodic terms of the ELP-2000/82 lunar
//! theory as given by J. Meeus, *Astronomical Algorithms* (2nd ed.),
//! chapter 47 (tables 47.A and 47.B). Browser-free.
//!
//! Meeus's series gives geocentric ecliptic longitude, latitude and distance
//! referred to the mean equinox of the date, to about 10 arcseconds. Here
//! they are moved to J2000 (the frame of the star catalog and of
//! `ephemeris`) by subtracting the general precession in longitude, which
//! leaves an error of a few arcseconds per decade away from 2000, and
//! turned into a J2000 equatorial vector. Tests compare with JPL Horizons
//! from 1900 to 2100 and with Meeus's worked example 47.a.

use crate::ephemeris::ecliptic_to_equatorial;

/// Terms of table 47.A: multiples of D, M, M', F, then the longitude
/// coefficient (1e-6 degrees) and the distance coefficient (1e-3 km).
const LR: [(i8, i8, i8, i8, i32, i32); 60] = [
    (0, 0, 1, 0, 6288774, -20905355),
    (2, 0, -1, 0, 1274027, -3699111),
    (2, 0, 0, 0, 658314, -2955968),
    (0, 0, 2, 0, 213618, -569925),
    (0, 1, 0, 0, -185116, 48888),
    (0, 0, 0, 2, -114332, -3149),
    (2, 0, -2, 0, 58793, 246158),
    (2, -1, -1, 0, 57066, -152138),
    (2, 0, 1, 0, 53322, -170733),
    (2, -1, 0, 0, 45758, -204586),
    (0, 1, -1, 0, -40923, -129620),
    (1, 0, 0, 0, -34720, 108743),
    (0, 1, 1, 0, -30383, 104755),
    (2, 0, 0, -2, 15327, 10321),
    (0, 0, 1, 2, -12528, 0),
    (0, 0, 1, -2, 10980, 79661),
    (4, 0, -1, 0, 10675, -34782),
    (0, 0, 3, 0, 10034, -23210),
    (4, 0, -2, 0, 8548, -21636),
    (2, 1, -1, 0, -7888, 24208),
    (2, 1, 0, 0, -6766, 30824),
    (1, 0, -1, 0, -5163, -8379),
    (1, 1, 0, 0, 4987, -16675),
    (2, -1, 1, 0, 4036, -12831),
    (2, 0, 2, 0, 3994, -10445),
    (4, 0, 0, 0, 3861, -11650),
    (2, 0, -3, 0, 3665, 14403),
    (0, 1, -2, 0, -2689, -7003),
    (2, 0, -1, 2, -2602, 0),
    (2, -1, -2, 0, 2390, 10056),
    (1, 0, 1, 0, -2348, 6322),
    (2, -2, 0, 0, 2236, -9884),
    (0, 1, 2, 0, -2120, 5751),
    (0, 2, 0, 0, -2069, 0),
    (2, -2, -1, 0, 2048, -4950),
    (2, 0, 1, -2, -1773, 4130),
    (2, 0, 0, 2, -1595, 0),
    (4, -1, -1, 0, 1215, -3958),
    (0, 0, 2, 2, -1110, 0),
    (3, 0, -1, 0, -892, 3258),
    (2, 1, 1, 0, -810, 2616),
    (4, -1, -2, 0, 759, -1897),
    (0, 2, -1, 0, -713, -2117),
    (2, 2, -1, 0, -700, 2354),
    (2, 1, -2, 0, 691, 0),
    (2, -1, 0, -2, 596, 0),
    (4, 0, 1, 0, 549, -1423),
    (0, 0, 4, 0, 537, -1117),
    (4, -1, 0, 0, 520, -1571),
    (1, 0, -2, 0, -487, -1739),
    (2, 1, 0, -2, -399, 0),
    (0, 0, 2, -2, -381, -4421),
    (1, 1, 1, 0, 351, 0),
    (3, 0, -2, 0, -340, 0),
    (4, 0, -3, 0, 330, 0),
    (2, -1, 2, 0, 327, 0),
    (0, 2, 1, 0, -323, 1165),
    (1, 1, -1, 0, 299, 0),
    (2, 0, 3, 0, 294, 0),
    (2, 0, -1, -2, 0, 8752),
];

/// Terms of table 47.B: multiples of D, M, M', F, then the latitude
/// coefficient (1e-6 degrees).
const B: [(i8, i8, i8, i8, i32); 60] = [
    (0, 0, 0, 1, 5128122),
    (0, 0, 1, 1, 280602),
    (0, 0, 1, -1, 277693),
    (2, 0, 0, -1, 173237),
    (2, 0, -1, 1, 55413),
    (2, 0, -1, -1, 46271),
    (2, 0, 0, 1, 32573),
    (0, 0, 2, 1, 17198),
    (2, 0, 1, -1, 9266),
    (0, 0, 2, -1, 8822),
    (2, -1, 0, -1, 8216),
    (2, 0, -2, -1, 4324),
    (2, 0, 1, 1, 4200),
    (2, 1, 0, -1, -3359),
    (2, -1, -1, 1, 2463),
    (2, -1, 0, 1, 2211),
    (2, -1, -1, -1, 2065),
    (0, 1, -1, -1, -1870),
    (4, 0, -1, -1, 1828),
    (0, 1, 0, 1, -1794),
    (0, 0, 0, 3, -1749),
    (0, 1, -1, 1, -1565),
    (1, 0, 0, 1, -1491),
    (0, 1, 1, 1, -1475),
    (0, 1, 1, -1, -1410),
    (0, 1, 0, -1, -1344),
    (1, 0, 0, -1, -1335),
    (0, 0, 3, 1, 1107),
    (4, 0, 0, -1, 1021),
    (4, 0, -1, 1, 833),
    (0, 0, 1, -3, 777),
    (4, 0, -2, 1, 671),
    (2, 0, 0, -3, 607),
    (2, 0, 2, -1, 596),
    (2, -1, 1, -1, 491),
    (2, 0, -2, 1, -451),
    (0, 0, 3, -1, 439),
    (2, 0, 2, 1, 422),
    (2, 0, -3, -1, 421),
    (2, 1, -1, 1, -366),
    (2, 1, 0, 1, -351),
    (4, 0, 0, 1, 331),
    (2, -1, 1, 1, 315),
    (2, -2, 0, -1, 302),
    (0, 0, 1, 3, -283),
    (2, 1, 1, -1, -229),
    (1, 1, 0, -1, 223),
    (1, 1, 0, 1, 223),
    (0, 1, -2, -1, -220),
    (2, 1, -1, -1, -220),
    (1, 0, 1, 1, -185),
    (2, -1, -2, -1, 181),
    (0, 1, 2, 1, -177),
    (4, 0, -2, -1, 176),
    (4, -1, -1, -1, 166),
    (1, 0, 1, -1, -164),
    (4, 0, 1, -1, 132),
    (1, 0, -1, -1, -119),
    (4, -1, 0, -1, 115),
    (2, -2, 0, 1, 107),
];

/// Mean equatorial radius of the Earth, km (for the topocentric shift).
pub const EARTH_RADIUS_KM: f64 = 6378.137;
/// Mean radius of the Moon, km.
pub const MOON_RADIUS_KM: f64 = 1737.4;

fn deg(a: f64) -> f64 {
    a.rem_euclid(360.0).to_radians()
}

/// Geocentric ecliptic longitude and latitude (degrees) referred to the mean
/// equinox of date, and distance (km), at `t` Julian centuries (TT) from
/// J2000. `None` for a non-finite time.
pub fn of_date(t: f64) -> Option<(f64, f64, f64)> {
    if !t.is_finite() || t.abs() > 30.0 {
        return None;
    }
    let (t2, t3, t4) = (t * t, t * t * t, t * t * t * t);
    let lp = 218.316_447_7 + 481_267.881_234_21 * t - 0.001_578_6 * t2 + t3 / 538_841.0
        - t4 / 65_194_000.0;
    let d = 297.850_192_1 + 445_267.111_403_4 * t - 0.001_881_9 * t2 + t3 / 545_868.0
        - t4 / 113_065_000.0;
    let m = 357.529_109_2 + 35_999.050_290_9 * t - 0.000_153_6 * t2 + t3 / 24_490_000.0;
    let mp = 134.963_396_4 + 477_198.867_505_5 * t + 0.008_741_4 * t2 + t3 / 69_699.0
        - t4 / 14_712_000.0;
    let f = 93.272_095_0 + 483_202.017_523_3 * t - 0.003_653_9 * t2 - t3 / 3_526_000.0
        + t4 / 863_310_000.0;
    let a1 = 119.75 + 131.849 * t;
    let a2 = 53.09 + 479_264.290 * t;
    let a3 = 313.45 + 481_266.484 * t;
    // The eccentricity of the Earth's orbit shrinks: terms with M scale by E.
    let e = 1.0 - 0.002_516 * t - 0.000_007_4 * t2;
    let ecc = |m_mult: i8| match m_mult.abs() {
        1 => e,
        2 => e * e,
        _ => 1.0,
    };
    let arg = |dd: i8, mm: i8, mpm: i8, ff: i8| {
        deg(dd as f64 * d + mm as f64 * m + mpm as f64 * mp + ff as f64 * f)
    };
    let (mut sl, mut sr, mut sb) = (0.0, 0.0, 0.0);
    for &(dd, mm, mpm, ff, l, r) in &LR {
        let a = arg(dd, mm, mpm, ff);
        sl += l as f64 * ecc(mm) * a.sin();
        sr += r as f64 * ecc(mm) * a.cos();
    }
    for &(dd, mm, mpm, ff, b) in &B {
        sb += b as f64 * ecc(mm) * arg(dd, mm, mpm, ff).sin();
    }
    sl += 3958.0 * deg(a1).sin() + 1962.0 * deg(lp - f).sin() + 318.0 * deg(a2).sin();
    sb += -2235.0 * deg(lp).sin()
        + 382.0 * deg(a3).sin()
        + 175.0 * deg(a1 - f).sin()
        + 175.0 * deg(a1 + f).sin()
        + 127.0 * deg(lp - mp).sin()
        - 115.0 * deg(lp + mp).sin();
    let lambda = (lp + sl / 1e6).rem_euclid(360.0);
    let beta = sb / 1e6;
    let dist = 385_000.56 + sr / 1000.0;
    Some((lambda, beta, dist))
}

/// Geocentric position of the Moon, km, in the J2000 equatorial frame (x
/// towards the equinox, z towards the celestial pole, as in `ephemeris`).
pub fn geocentric_equatorial(t: f64) -> Option<[f64; 3]> {
    let (lambda, beta, dist) = of_date(t)?;
    // General precession in longitude (IAU 1976), back to the J2000 equinox.
    let p = (5029.0966 * t + 1.111_13 * t * t) / 3600.0;
    let (l, b) = ((lambda - p).to_radians(), beta.to_radians());
    Some(ecliptic_to_equatorial([
        dist * b.cos() * l.cos(),
        dist * b.cos() * l.sin(),
        dist * b.sin(),
    ]))
}

/// The Moon as seen from a place on the Earth: geocentric position minus the
/// observer's (on a sphere of the equatorial radius; the flattening moves
/// the Moon by under 3 arcseconds). `lst` is the local sidereal time and
/// `lat` the latitude, radians; same frame and units as
/// [`geocentric_equatorial`].
pub fn topocentric_equatorial(t: f64, lat: f64, lst: f64) -> Option<[f64; 3]> {
    let g = geocentric_equatorial(t)?;
    let o = [
        EARTH_RADIUS_KM * lat.cos() * lst.cos(),
        EARTH_RADIUS_KM * lat.cos() * lst.sin(),
        EARTH_RADIUS_KM * lat.sin(),
    ];
    Some([g[0] - o[0], g[1] - o[1], g[2] - o[2]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ephemeris::centuries_from_jd;

    /// JPL Horizons (DE441) geocentric geometric position of the Moon
    /// (301), ICRF equatorial, km, at Julian dates in TDB. Query:
    /// EPHEM_TYPE=VECTORS, CENTER='500@399', REF_PLANE=FRAME,
    /// REF_SYSTEM=ICRF, VEC_CORR=NONE, fetched 2026-11-20.
    #[allow(clippy::excessive_precision)] // copied verbatim from Horizons
    const HORIZONS: [[f64; 4]; 12] = [
        [
            2415020.5,
            2.446493173227516E+04,
            -3.399842486952020E+05,
            -1.397252435346273E+05,
        ],
        [
            2433282.5,
            1.865116707714126E+05,
            3.128367906842186E+05,
            1.644024401574678E+05,
        ],
        [
            2440000.25,
            3.677563052629798E+05,
            1.513938124075382E+05,
            6.972467506472474E+04,
        ],
        [
            2444240.8,
            -7.020150679829941E+04,
            3.613391814510769E+05,
            1.275443723957360E+05,
        ],
        [
            2448724.5,
            -2.521187123185933E+05,
            2.539745064863941E+05,
            8.749713651676469E+04,
        ],
        [
            2451545.0,
            -2.916083841877129E+05,
            -2.667168338540655E+05,
            -7.610248730658794E+04,
        ],
        [
            2455197.5,
            -8.137643630085903E+04,
            3.193181848733827E+05,
            1.433837962568061E+05,
        ],
        [
            2457388.75,
            -4.032240834940751E+05,
            2.472657501585437E+03,
            4.320409662534539E+03,
        ],
        [
            2460310.5,
            -3.679525311419403E+05,
            1.427749731429682E+05,
            8.934228118085049E+04,
        ],
        [
            2461000.3,
            -1.777933454553749E+05,
            -3.200746577144813E+05,
            -1.766836640741577E+05,
        ],
        [
            2466154.5,
            -1.744411495543004E+05,
            3.241622906665963E+05,
            1.681131783434096E+05,
        ],
        [
            2488069.5,
            -3.395196032258248E+05,
            1.358668148543692E+05,
            6.660210198127017E+04,
        ],
    ];

    fn norm(v: [f64; 3]) -> f64 {
        (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
    }

    /// Angle between two vectors, accurate when they are nearly parallel.
    fn angle_deg(a: [f64; 3], b: [f64; 3]) -> f64 {
        let d: f64 = (0..3).map(|i| a[i] * b[i]).sum();
        let c = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        norm(c).atan2(d).to_degrees()
    }

    #[test]
    fn meeus_example_47a() {
        // 1992 April 12, 0h TD: lambda 133.162655, beta -3.229126,
        // distance 368409.7 km.
        let (l, b, d) = of_date(centuries_from_jd(2_448_724.5)).unwrap();
        assert!((l - 133.162_655).abs() < 1e-4, "{l}");
        assert!((b + 3.229_126).abs() < 1e-4, "{b}");
        assert!((d - 368_409.7).abs() < 0.5, "{d}");
    }

    #[test]
    fn matches_horizons_1900_to_2100() {
        let mut worst = (0.0f64, 0.0f64);
        for [jd, x, y, z] in HORIZONS {
            let got = geocentric_equatorial(centuries_from_jd(jd)).unwrap();
            let want = [x, y, z];
            let a = angle_deg(got, want);
            let r = (norm(got) / norm(want) - 1.0).abs();
            worst = (worst.0.max(a), worst.1.max(r));
        }
        // Measured: 0.013 degrees (a fortieth of the Moon's diameter) and
        // 5 km; most of it is the precession shortcut at 1900 and 2100.
        assert!(worst.0 < 0.015, "direction off by {} degrees", worst.0);
        assert!(worst.1 < 2e-5, "distance off by {}", worst.1);
    }

    #[test]
    fn parallax_shifts_the_moon_by_up_to_a_degree() {
        let t = centuries_from_jd(2_451_545.0);
        let g = geocentric_equatorial(t).unwrap();
        // An observer with the Moon on the horizon sees it shifted most;
        // one with it overhead not at all.
        let gn = norm(g);
        let up = [g[0] / gn, g[1] / gn, g[2] / gn];
        let lat_up = up[2].asin();
        let lst_up = up[1].atan2(up[0]);
        let overhead = topocentric_equatorial(t, lat_up, lst_up).unwrap();
        assert!(angle_deg(overhead, g) < 1e-6);
        assert!((norm(g) - norm(overhead) - EARTH_RADIUS_KM).abs() < 1e-6);
        let side = topocentric_equatorial(t, 0.0, lst_up + std::f64::consts::FRAC_PI_2).unwrap();
        let shift = angle_deg(side, g);
        assert!(shift > 0.85 && shift < 1.0, "{shift}");
    }

    #[test]
    fn no_position_for_bad_times() {
        assert_eq!(of_date(f64::NAN), None);
        assert_eq!(geocentric_equatorial(f64::INFINITY), None);
        assert_eq!(of_date(100.0), None);
    }
}
