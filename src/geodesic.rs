//! Inverse geodesic problem on the WGS84 ellipsoid: the length of the
//! shortest path between two points. Pure and browser-free.
//!
//! Most pairs use Vincenty's iteration (about 0.1 mm accurate). Vincenty
//! fails to converge near the antipode, so pairs more than 150 degrees apart
//! use Karney's formulation instead (C. F. F. Karney, "Algorithms for
//! geodesics", J. Geodesy 87, 43-55, 2013): the geodesic is parameterized
//! by its azimuth at the first point, the longitude difference and length
//! come from numerically integrated auxiliary-sphere integrals, and the
//! azimuth is found by bisection. That path always converges. Meridians
//! (including through a pole) are integrated directly.

use std::f64::consts::PI;

/// WGS84 semi-major axis, metres.
pub const A: f64 = 6_378_137.0;
/// WGS84 flattening.
pub const F: f64 = 1.0 / 298.257_223_563;
const B: f64 = A * (1.0 - F);
/// Second eccentricity squared, `(a^2 - b^2) / b^2`.
const EP2: f64 = F * (2.0 - F) / ((1.0 - F) * (1.0 - F));
/// Cosine of the central angle (150 degrees) above which Vincenty is not used.
const VINCENTY_COS_LIMIT: f64 = -0.866_025_403_784_438_6;
/// Within this cosine of latitude (about 6 micrometres of arc) a point
/// counts as a pole, where longitude means nothing.
const POLE_EPS: f64 = 1e-12;

/// Gauss-Legendre 5-point nodes and weights on [-1, 1] (half of them).
const GL_X: [f64; 3] = [0.0, 0.538_469_310_105_683_1, 0.906_179_845_938_664];
const GL_W: [f64; 3] = [
    0.568_888_888_888_888_9,
    0.478_628_670_499_366_5,
    0.236_926_885_056_189_1,
];

/// Integral of `f` over `[a, b]`: composite 5-point Gauss-Legendre on panels
/// of at most pi/8, which is exact to rounding for the smooth integrands here.
fn integrate(f: impl Fn(f64) -> f64, a: f64, b: f64) -> f64 {
    let n = (((b - a).abs() / (PI / 8.0)).ceil() as usize).max(1);
    let h = (b - a) / n as f64;
    let half = 0.5 * h;
    let mut sum = 0.0;
    for i in 0..n {
        let mid = a + (i as f64 + 0.5) * h;
        let mut s = GL_W[0] * f(mid);
        for k in 1..3 {
            s += GL_W[k] * (f(mid - half * GL_X[k]) + f(mid + half * GL_X[k]));
        }
        sum += s * half;
    }
    sum
}

/// Meridian arc from the equator to reduced latitude `beta`, in units of `b`.
fn meridian_arc(beta: f64) -> f64 {
    integrate(|s| (1.0 + EP2 * s.sin().powi(2)).sqrt(), 0.0, beta)
}

/// A point pair in Karney's canonical frame: reduced latitudes as
/// (sin, cos), with `|lat1| >= |lat2|`, `lat1 <= 0`, and the longitude
/// difference in `[0, pi]`.
struct Canon {
    sb1: f64,
    cb1: f64,
    sb2: f64,
    cb2: f64,
    lon12: f64,
}

impl Canon {
    fn new(a: (f64, f64), b: (f64, f64)) -> Canon {
        let half = PI / 2.0;
        let (mut lat1, mut lat2) = (a.0.clamp(-half, half), b.0.clamp(-half, half));
        // Longitude difference folded into [0, pi].
        let mut lon12 = (b.1 - a.1).rem_euclid(2.0 * PI);
        if lon12 > PI {
            lon12 = 2.0 * PI - lon12;
        }
        if lat1.abs() < lat2.abs() {
            std::mem::swap(&mut lat1, &mut lat2);
        }
        // Negating an equatorial lat1 gives -0.0, which atan2 relies on.
        let sign = if lat1 < 0.0 { 1.0 } else { -1.0 };
        lat1 *= sign;
        lat2 *= sign;
        if lat1.cos().abs() < POLE_EPS || lat2.cos().abs() < POLE_EPS {
            lon12 = 0.0;
        }
        let reduced = |lat: f64| {
            let (s, c) = ((1.0 - F) * lat.sin(), lat.cos());
            let n = s.hypot(c);
            (s / n, c / n)
        };
        let (sb1, cb1) = reduced(lat1);
        let (sb2, cb2) = reduced(lat2);
        Canon {
            sb1,
            cb1,
            sb2,
            cb2,
            lon12,
        }
    }
}

/// Shortest distance in metres between two points given as (lat, lon) in
/// radians, on the WGS84 ellipsoid. `None` for non-finite input or a
/// latitude beyond a pole.
pub fn distance_m(a: (f64, f64), b: (f64, f64)) -> Option<f64> {
    let valid = |p: (f64, f64)| p.0.is_finite() && p.1.is_finite() && p.0.abs() <= PI / 2.0 + 1e-9;
    if !valid(a) || !valid(b) {
        return None;
    }
    let c = Canon::new(a, b);
    if c.lon12 == 0.0 || c.lon12 == PI {
        let (b1, b2) = (c.sb1.atan2(c.cb1), c.sb2.atan2(c.cb2));
        return Some(if c.lon12 == 0.0 {
            B * (meridian_arc(b2) - meridian_arc(b1)).abs()
        } else {
            // Over the nearer pole (the south one in canonical form).
            B * (2.0 * meridian_arc(PI / 2.0) + meridian_arc(b1) + meridian_arc(b2))
        });
    }
    let cos_sigma = c.sb1 * c.sb2 + c.cb1 * c.cb2 * c.lon12.cos();
    if cos_sigma > VINCENTY_COS_LIMIT {
        if let Some(s) = vincenty(&c) {
            return Some(s);
        }
    }
    Some(karney(&c))
}

/// Vincenty's inverse formula. `None` if it does not converge.
fn vincenty(c: &Canon) -> Option<f64> {
    let Canon {
        sb1: su1,
        cb1: cu1,
        sb2: su2,
        cb2: cu2,
        lon12: l,
    } = *c;
    let mut lam = l;
    for _ in 0..100 {
        let (sl, cl) = lam.sin_cos();
        let ss = (cu2 * sl).hypot(cu1 * su2 - su1 * cu2 * cl);
        let cs = su1 * su2 + cu1 * cu2 * cl;
        if ss == 0.0 {
            return if cs > 0.0 { Some(0.0) } else { None };
        }
        let sigma = ss.atan2(cs);
        let sa = cu1 * cu2 * sl / ss;
        let c2a = 1.0 - sa * sa;
        let c2sm = if c2a != 0.0 {
            cs - 2.0 * su1 * su2 / c2a
        } else {
            0.0
        };
        let k = F / 16.0 * c2a * (4.0 + F * (4.0 - 3.0 * c2a));
        let next = l
            + (1.0 - k) * F * sa * (sigma + k * ss * (c2sm + k * cs * (-1.0 + 2.0 * c2sm * c2sm)));
        if !next.is_finite() || next.abs() > PI {
            return None;
        }
        if (next - lam).abs() < 1e-14 {
            let u2 = c2a * EP2;
            let ka = 1.0 + u2 / 16384.0 * (4096.0 + u2 * (-768.0 + u2 * (320.0 - 175.0 * u2)));
            let kb = u2 / 1024.0 * (256.0 + u2 * (-128.0 + u2 * (74.0 - 47.0 * u2)));
            let ds = kb
                * ss
                * (c2sm
                    + kb / 4.0
                        * (cs * (-1.0 + 2.0 * c2sm * c2sm)
                            - kb / 6.0
                                * c2sm
                                * (-3.0 + 4.0 * ss * ss)
                                * (-3.0 + 4.0 * c2sm * c2sm)));
            return Some(B * ka * (sigma - ds));
        }
        lam = next;
    }
    None
}

/// Auxiliary-sphere quantities of the geodesic leaving the first point at
/// azimuth `alp1` (in the canonical frame it arrives at the second point
/// heading north or east, `cos alp2 >= 0`).
struct Leg {
    k2: f64,
    sig1: f64,
    sig2: f64,
    /// Longitude difference reached, radians.
    lam12: f64,
}

fn leg(c: &Canon, alp1: f64) -> Leg {
    let Canon {
        sb1, cb1, sb2, cb2, ..
    } = *c;
    let (salp1, calp1) = alp1.sin_cos();
    let salp0 = salp1 * cb1;
    let calp0 = calp1.hypot(salp1 * sb1);
    let calp2 = ((calp1 * cb1).powi(2) + (cb2 - cb1) * (cb2 + cb1))
        .max(0.0)
        .sqrt()
        / cb2;
    let (sig1, sig2) = (sb1.atan2(calp1 * cb1), sb2.atan2(calp2 * cb2));
    let om1 = (salp0 * sb1).atan2(calp1 * cb1);
    let om2 = (salp0 * sb2).atan2(calp2 * cb2);
    let k2 = EP2 * calp0 * calp0;
    let i3 = integrate(
        |s| (2.0 - F) / (1.0 + (1.0 - F) * (1.0 + k2 * s.sin().powi(2)).sqrt()),
        sig1,
        sig2,
    );
    Leg {
        k2,
        sig1,
        sig2,
        lam12: (om2 - om1) - F * salp0 * i3,
    }
}

/// Karney's method: bisect the starting azimuth so the longitude difference
/// matches, then integrate the length. Needs `0 < lon12 < pi` and neither
/// point at a pole.
fn karney(c: &Canon) -> f64 {
    let equatorial = c.sb1 == 0.0 && c.sb2 == 0.0;
    if equatorial && c.lon12 <= PI * (1.0 - F) {
        return A * c.lon12; // along the equator
    }
    // Both on the equator: azimuths below 90 degrees have no solution.
    let (mut lo, mut hi) = (if equatorial { PI / 2.0 } else { 0.0 }, PI);
    let mut alp = 0.5 * (lo + hi);
    for _ in 0..200 {
        alp = 0.5 * (lo + hi);
        if leg(c, alp).lam12 < c.lon12 {
            lo = alp;
        } else {
            hi = alp;
        }
        if hi - lo < 1e-16 {
            break;
        }
    }
    let l = leg(c, alp);
    B * integrate(|s| (1.0 + l.k2 * s.sin().powi(2)).sqrt(), l.sig1, l.sig2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        distance_m(
            (lat1.to_radians(), lon1.to_radians()),
            (lat2.to_radians(), lon2.to_radians()),
        )
        .unwrap()
    }

    /// Reference values below are from GeographicLib
    /// (`Geodesic.WGS84.Inverse(...)['s12']`).
    #[track_caller]
    fn check(got: f64, want: f64, tol: f64) {
        assert!((got - want).abs() <= tol, "got {got}, want {want}");
    }

    #[test]
    fn meridian_and_equator_quarters() {
        check(d(0.0, 0.0, 90.0, 0.0), 10_001_965.729, 1e-3);
        check(d(0.0, 0.0, -90.0, 0.0), 10_001_965.729, 1e-3);
        check(d(0.0, 0.0, 0.0, 90.0), 10_018_754.171, 1e-3);
        check(d(0.0, 10.0, 0.0, 100.0), 10_018_754.171, 1e-3);
    }

    #[test]
    fn city_pairs() {
        check(
            d(40.6413, -73.7781, 51.4700, -0.4543),
            5_554_908.790_5,
            1e-3,
        );
        check(
            d(35.6762, 139.6503, -33.8688, 151.2093),
            7_792_174.827_1,
            1e-3,
        );
    }

    #[test]
    fn nearly_antipodal_pairs() {
        // The classic cases where Vincenty fails to converge.
        check(d(0.0, 0.0, 0.5, 179.5), 19_936_288.579_0, 1e-3);
        check(d(0.0, 0.0, 0.5, 179.7), 19_944_127.420_8, 1e-3);
        check(d(0.0, 0.0, 0.0, 179.9), 20_003_008.421_5, 1e-3);
        check(d(-30.0, 0.0, 30.0, 179.8), 20_000_239.437_7, 1e-3);
        check(d(1.0, 0.0, -1.0, 179.99), 20_003_922.228_1, 1e-3);
    }

    #[test]
    fn exactly_antipodal_goes_over_the_pole() {
        check(d(0.0, 0.0, 0.0, 180.0), 20_003_931.458_6, 1e-3);
        check(d(30.0, 0.0, -30.0, 180.0), 20_003_931.458_6, 1e-3);
        check(d(30.0, 20.0, -30.0, -160.0), 20_003_931.458_6, 1e-3);
    }

    #[test]
    fn poles() {
        check(d(90.0, 0.0, -90.0, 0.0), 20_003_931.458_6, 1e-3);
        check(d(90.0, 10.0, -90.0, 130.0), 20_003_931.458_6, 1e-3);
        check(d(90.0, 0.0, 90.0, 123.0), 0.0, 1e-6);
        // Longitude is meaningless at a pole.
        check(d(90.0, 0.0, 0.0, 77.0), 10_001_965.729, 1e-3);
        check(d(90.0, 0.0, 0.0, -150.0), 10_001_965.729, 1e-3);
    }

    #[test]
    fn coincident_and_tiny() {
        assert_eq!(d(12.0, 34.0, 12.0, 34.0), 0.0);
        assert_eq!(d(0.0, 0.0, 0.0, 0.0), 0.0);
        check(d(10.0, 20.0, 10.0, 20.000001), 0.1096, 1e-4);
    }

    #[test]
    fn seam_and_symmetry() {
        check(d(0.0, 179.0, 0.0, -179.0), 222_638.981_6, 1e-3);
        check(d(0.0, 179.0, 0.0, 181.0), 222_638.981_6, 1e-3);
        for &(a, b, c, e) in &[
            (10.0, 20.0, -30.0, 100.0),
            (0.0, 0.0, 0.5, 179.5),
            (60.0, -170.0, -45.0, 10.0),
        ] {
            check(d(a, b, c, e), d(c, e, a, b), 1e-6);
        }
    }

    #[test]
    fn vincenty_and_karney_agree() {
        // Both solvers on pairs where either works.
        for &(a, b, c, e) in &[
            (51.5_f64, -0.1_f64, 40.7_f64, -74.0_f64),
            (-33.9, 151.2, 35.7, 139.7),
            (10.0, 20.0, 12.0, 25.0),
            (80.0, 10.0, -20.0, 100.0),
            (0.0, 0.0, 0.0, 60.0),
            (0.0, 0.0, 30.0, 100.0),
        ] {
            let canon = Canon::new(
                (a.to_radians(), b.to_radians()),
                (c.to_radians(), e.to_radians()),
            );
            check(vincenty(&canon).unwrap(), karney(&canon), 5e-4);
        }
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(distance_m((bad, 0.0), (0.0, 0.0)), None);
            assert_eq!(distance_m((0.0, 0.0), (0.0, bad)), None);
        }
        assert_eq!(distance_m((2.0, 0.0), (0.0, 0.0)), None);
    }

    #[test]
    fn close_to_the_sphere() {
        // Within the 0.5% the spherical formula promised.
        let sphere = 6371.0e3 * 90.0_f64.to_radians();
        assert!((d(0.0, 0.0, 0.0, 90.0) / sphere - 1.0).abs() < 0.005);
    }
}
