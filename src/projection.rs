//! Map projections: forward and inverse math for the flat projections, the
//! `Projection` enum shared by the camera, the URL hash and the shaders, and
//! the globe <-> flat morph timeline. Browser-free, so it is unit tested
//! natively.
//!
//! A flat map is described in "plane units": the sphere has radius 1, `x` is
//! measured from the central meridian (the camera's longitude), and `y` from
//! the equator, north up. `forward` takes a latitude and a longitude
//! relative to the central meridian (radians) and `inverse` undoes it.
//!
//! `src/shaders/globe.frag` (inverse), `src/shaders/morph.vert` and
//! `src/shaders/line.vert` (forward) implement the same math in GLSL and the
//! tests at the bottom of this file check that their constants and
//! polynomial tables are identical to the ones here.

use std::f64::consts::{FRAC_PI_2, PI};

/// Mercator is clamped at this latitude (about 85.0511 degrees), where the
/// map becomes a square: `y` there is exactly π.
pub const MERCATOR_MAX_LAT: f64 = 1.484_422_229_745_332_4;
/// Largest `y` of the Robinson map (the poles).
pub const ROBINSON_Y_SCALE: f64 = 1.3523;
/// Robinson `x` scale: the equator is `0.8487 * λ` wide.
pub const ROBINSON_X_SCALE: f64 = 0.8487;
/// Robinson table, every 5 degrees of latitude from the equator to the pole.
/// `PLEN` (relative parallel length) and `PDFE` (distance from the equator).
pub const ROBINSON_PLEN: [f64; 19] = [
    1.0000, 0.9986, 0.9954, 0.9900, 0.9822, 0.9730, 0.9600, 0.9427, 0.9216, 0.8962, 0.8679, 0.8350,
    0.7986, 0.7597, 0.7186, 0.6732, 0.6213, 0.5722, 0.5322,
];
pub const ROBINSON_PDFE: [f64; 19] = [
    0.0000, 0.0620, 0.1240, 0.1860, 0.2480, 0.3100, 0.3720, 0.4340, 0.4958, 0.5571, 0.6176, 0.6769,
    0.7346, 0.7903, 0.8435, 0.8936, 0.9394, 0.9761, 1.0000,
];
/// Mollweide: `x = MOLLWEIDE_X * λ * cos θ`, `y = √2 sin θ`.
pub const MOLLWEIDE_X: f64 = 0.900_316_316_157_106_1; // 2 √2 / π
/// Newton steps for the Mollweide auxiliary angle. The GLSL uses the same
/// count (checked by the tests); near the poles convergence is only linear,
/// which is invisible because the error is multiplied by cos θ.
#[cfg(test)]
pub const MOLLWEIDE_ITERATIONS: usize = 14;
/// Flat maps are drawn this much smaller than the globe at the same camera
/// altitude, so zooming all the way out shows the whole world.
pub const FLAT_ZOOM: f64 = 1.0 / 3.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Projection {
    #[default]
    Globe,
    Equirectangular,
    Mercator,
    Robinson,
    Mollweide,
}

impl Projection {
    pub const ALL: [Projection; 5] = [
        Projection::Globe,
        Projection::Equirectangular,
        Projection::Mercator,
        Projection::Robinson,
        Projection::Mollweide,
    ];

    /// Identifier used in the URL (`&proj=`) and in element values.
    pub fn id(self) -> &'static str {
        match self {
            Projection::Globe => "globe",
            Projection::Equirectangular => "equirect",
            Projection::Mercator => "mercator",
            Projection::Robinson => "robinson",
            Projection::Mollweide => "mollweide",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Projection::Globe => "Globe",
            Projection::Equirectangular => "Equirectangular",
            Projection::Mercator => "Mercator",
            Projection::Robinson => "Robinson",
            Projection::Mollweide => "Mollweide",
        }
    }

    pub fn from_id(id: &str) -> Option<Projection> {
        Projection::ALL.into_iter().find(|p| p.id() == id.trim())
    }

    /// Value of the projection id uniforms in the shaders (0 = globe).
    pub fn shader_index(self) -> i32 {
        match self {
            Projection::Globe => 0,
            Projection::Equirectangular => 1,
            Projection::Mercator => 2,
            Projection::Robinson => 3,
            Projection::Mollweide => 4,
        }
    }

    pub fn is_flat(self) -> bool {
        self != Projection::Globe
    }

    /// The next projection in `ALL`, wrapping around.
    pub fn next(self) -> Projection {
        let i = Projection::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Projection::ALL[(i + 1) % Projection::ALL.len()]
    }

    /// The previous projection in `ALL`, wrapping around.
    pub fn prev(self) -> Projection {
        let n = Projection::ALL.len();
        let i = Projection::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Projection::ALL[(i + n - 1) % n]
    }

    /// The world repeats left and right (equirectangular, Mercator). The
    /// others draw a single world inside their outline.
    pub fn repeats(self) -> bool {
        matches!(self, Projection::Equirectangular | Projection::Mercator)
    }

    /// Largest `|y|` of the map; the map ends there. 0 for the globe.
    pub fn y_max(self) -> f64 {
        match self {
            Projection::Globe => 0.0,
            Projection::Equirectangular => FRAC_PI_2,
            Projection::Mercator => PI,
            Projection::Robinson => ROBINSON_Y_SCALE,
            Projection::Mollweide => std::f64::consts::SQRT_2,
        }
    }

    /// Latitude beyond which the map shows nothing (Mercator only).
    pub fn max_lat(self) -> f64 {
        match self {
            Projection::Mercator => MERCATOR_MAX_LAT,
            _ => FRAC_PI_2,
        }
    }

    /// Widest `|x|` of the map at the equator for one world (π times the
    /// equatorial scale). Used to size the world on screen.
    pub fn half_width(self) -> f64 {
        match self {
            Projection::Robinson => PI * ROBINSON_X_SCALE,
            Projection::Mollweide => PI * MOLLWEIDE_X,
            _ => PI,
        }
    }
}

/// Linear interpolation of a Robinson table at `lat` (radians, any sign).
fn robinson_table(table: &[f64; 19], lat: f64) -> f64 {
    let a = (lat.abs() / FRAC_PI_2 * 18.0).clamp(0.0, 18.0);
    let i = (a.floor() as usize).min(17);
    let f = a - i as f64;
    table[i] + (table[i + 1] - table[i]) * f
}

/// Mollweide auxiliary angle θ with `2θ + sin 2θ = π sin φ`.
pub fn mollweide_theta(lat: f64) -> f64 {
    let target = PI * lat.sin();
    let mut th = lat;
    for _ in 0..100 {
        let d = 2.0 + 2.0 * (2.0 * th).cos();
        if d.abs() < 1e-12 {
            break;
        }
        let step = (2.0 * th + (2.0 * th).sin() - target) / d;
        th -= step;
        if step.abs() < 1e-15 {
            break;
        }
    }
    th
}

/// Plane position of a point at latitude `lat` and longitude `lon` relative
/// to the central meridian (radians). `None` for the globe or NaN input.
/// Mercator clamps the latitude to ±[`MERCATOR_MAX_LAT`], so the poles land
/// on the map's top and bottom edge; use [`in_domain`] to tell them apart.
pub fn forward(p: Projection, lat: f64, lon: f64) -> Option<(f64, f64)> {
    if !(lat.is_finite() && lon.is_finite()) {
        return None;
    }
    let lat = lat.clamp(-FRAC_PI_2, FRAC_PI_2);
    Some(match p {
        Projection::Globe => return None,
        Projection::Equirectangular => (lon, lat),
        Projection::Mercator => {
            let l = lat.clamp(-MERCATOR_MAX_LAT, MERCATOR_MAX_LAT);
            (lon, (PI / 4.0 + l / 2.0).tan().ln())
        }
        Projection::Robinson => (
            ROBINSON_X_SCALE * robinson_table(&ROBINSON_PLEN, lat) * lon,
            ROBINSON_Y_SCALE * robinson_table(&ROBINSON_PDFE, lat) * lat.signum(),
        ),
        Projection::Mollweide => {
            let th = mollweide_theta(lat);
            (
                MOLLWEIDE_X * lon * th.cos(),
                std::f64::consts::SQRT_2 * th.sin(),
            )
        }
    })
}

/// Whether the map shows this latitude at all (false past Mercator's limit).
pub fn in_domain(p: Projection, lat: f64) -> bool {
    p.is_flat() && lat.abs() <= p.max_lat() + 1e-12
}

/// Latitude and longitude relative to the central meridian for a plane
/// position, or `None` outside the map: past the top or bottom, or beyond
/// ±180° of the central meridian for the projections that draw one world.
pub fn inverse(p: Projection, x: f64, y: f64) -> Option<(f64, f64)> {
    if !(x.is_finite() && y.is_finite()) {
        return None;
    }
    let (lat, lon) = match p {
        Projection::Globe => return None,
        Projection::Equirectangular => {
            if y.abs() > FRAC_PI_2 {
                return None;
            }
            (y, x)
        }
        Projection::Mercator => {
            if y.abs() > PI {
                return None;
            }
            (2.0 * y.exp().atan() - FRAC_PI_2, x)
        }
        Projection::Robinson => {
            let v = y.abs() / ROBINSON_Y_SCALE;
            if v > 1.0 {
                return None;
            }
            let lat = robinson_lat_of_pdfe(v) * y.signum();
            let plen = robinson_table(&ROBINSON_PLEN, lat);
            (lat, x / (ROBINSON_X_SCALE * plen))
        }
        Projection::Mollweide => {
            let s = y / std::f64::consts::SQRT_2;
            if s.abs() > 1.0 {
                return None;
            }
            let th = s.asin();
            let lat = ((2.0 * th + (2.0 * th).sin()) / PI).clamp(-1.0, 1.0).asin();
            let c = th.cos();
            if c < 1e-12 {
                // The pole is a point: any longitude is on it (only x = 0).
                return (x.abs() < 1e-9).then_some((lat, 0.0));
            }
            (lat, x / (MOLLWEIDE_X * c))
        }
    };
    if !p.repeats() && lon.abs() > PI + 1e-12 {
        return None;
    }
    Some((lat, lon))
}

/// Latitude (radians, 0..π/2) whose Robinson `PDFE` is `v` (0..1).
fn robinson_lat_of_pdfe(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    let mut i = 0;
    while i < 17 && v > ROBINSON_PDFE[i + 1] {
        i += 1;
    }
    let (a, b) = (ROBINSON_PDFE[i], ROBINSON_PDFE[i + 1]);
    let f = if b > a { (v - a) / (b - a) } else { 0.0 };
    (i as f64 + f) / 18.0 * FRAC_PI_2
}

/// `y` of a latitude on the central meridian. Mercator clamps.
pub fn y_of_lat(p: Projection, lat: f64) -> f64 {
    forward(p, lat, 0.0).map_or(0.0, |(_, y)| y)
}

/// Latitude at plane height `y`, with `y` clamped onto the map.
pub fn lat_of_y(p: Projection, y: f64) -> f64 {
    let ym = p.y_max();
    let y = if y.is_finite() { y.clamp(-ym, ym) } else { 0.0 };
    inverse(p, 0.0, y).map_or(0.0, |(lat, _)| lat)
}

/// `∂x/∂λ` at a latitude: how fast the map moves sideways per radian of
/// longitude. 1 for the cylindrical projections.
pub fn x_scale(p: Projection, lat: f64) -> f64 {
    match p {
        Projection::Globe | Projection::Equirectangular | Projection::Mercator => 1.0,
        Projection::Robinson => ROBINSON_X_SCALE * robinson_table(&ROBINSON_PLEN, lat),
        Projection::Mollweide => MOLLWEIDE_X * mollweide_theta(lat).cos(),
    }
}

/// How much the map stretches along a parallel at `lat` relative to the
/// ground: `x_scale / cos φ`. 1 for equirectangular on the equator, `1/cos φ`
/// for Mercator. Capped near the poles, where it is unbounded or undefined.
pub fn parallel_stretch(p: Projection, lat: f64) -> f64 {
    let c = lat.cos().max(0.02);
    (x_scale(p, lat) / c).max(1e-3)
}

/// How the line pass must repeat the coast and border segments on a settled
/// flat map whose view reaches `half_width` plane units either side of the
/// central meridian: `(copies, clip)`. Cylindrical maps repeat, so every copy
/// of the world the view reaches is drawn (`copies` on each side of the
/// central one) and nothing is clipped. The others show one world inside an
/// outline: the copies on each side catch segments that cross the seam, and
/// `clip` cuts every copy at the outline's side edge.
pub fn line_wraps(p: Projection, half_width: f64) -> (i32, bool) {
    if p.repeats() {
        let k = ((half_width - PI) / (2.0 * PI)).ceil();
        (
            if k.is_nan() {
                0
            } else {
                k.clamp(0.0, 4.0) as i32
            },
            false,
        )
    } else {
        (1, true)
    }
}

/// Value of the morph timeline at `progress` in 0..1, eased.
pub fn ease(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Duration of a globe <-> flat morph, in milliseconds.
pub const MORPH_MS: f64 = 900.0;

/// What the renderer draws: projection `a` blended into `b` by `t`
/// (0 = all `a`, 1 = all `b`). Settled when `a == b`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blend {
    pub a: Projection,
    pub b: Projection,
    pub t: f64,
}

impl Blend {
    pub fn settled(p: Projection) -> Self {
        Self { a: p, b: p, t: 1.0 }
    }

    /// Not mid-morph: one projection is drawn exactly.
    pub fn is_settled(&self) -> bool {
        self.a == self.b || self.t >= 1.0 || self.t <= 0.0
    }

    /// The projection drawn exactly when settled.
    pub fn shown(&self) -> Projection {
        if self.a == self.b || self.t >= 1.0 {
            self.b
        } else {
            self.a
        }
    }

    /// Weight of the globe in the blend, 0..1 (1 = pure globe).
    pub fn globe_weight(&self) -> f64 {
        let wa = if self.a == Projection::Globe {
            1.0 - self.t
        } else {
            0.0
        };
        let wb = if self.b == Projection::Globe {
            self.t
        } else {
            0.0
        };
        (wa + wb).clamp(0.0, 1.0)
    }
}

/// Timeline of the morph between the projection that was shown and the one
/// just chosen.
#[derive(Clone, Copy, Debug)]
pub struct Morph {
    from: Projection,
    to: Projection,
    /// Linear progress, 0..1.
    progress: f64,
}

impl Morph {
    pub fn new(p: Projection) -> Self {
        Self {
            from: p,
            to: p,
            progress: 1.0,
        }
    }

    /// Start morphing toward `to`. With `instant` (reduced motion) the
    /// change is immediate. Choosing the target already shown does nothing.
    pub fn start(&mut self, to: Projection, instant: bool) {
        if to == self.to {
            return;
        }
        // Mid-morph, restart from whichever end is further along.
        self.from = if self.progress >= 0.5 {
            self.to
        } else {
            self.from
        };
        self.to = to;
        self.progress = if instant { 1.0 } else { 0.0 };
    }

    /// Advance by `dt` milliseconds; true if the picture changed.
    pub fn tick(&mut self, dt: f64) -> bool {
        if self.progress >= 1.0 || !dt.is_finite() {
            return false;
        }
        self.progress = (self.progress + dt.max(0.0) / MORPH_MS).min(1.0);
        true
    }

    pub fn animating(&self) -> bool {
        self.progress < 1.0
    }

    pub fn blend(&self) -> Blend {
        if self.progress >= 1.0 || self.from == self.to {
            Blend::settled(self.to)
        } else {
            Blend {
                a: self.from,
                b: self.to,
                t: ease(self.progress),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLATS: [Projection; 4] = [
        Projection::Equirectangular,
        Projection::Mercator,
        Projection::Robinson,
        Projection::Mollweide,
    ];

    fn deg(d: f64) -> f64 {
        d.to_radians()
    }

    #[test]
    fn ids_round_trip_and_shader_indices_are_unique() {
        for p in Projection::ALL {
            assert_eq!(Projection::from_id(p.id()), Some(p));
        }
        assert_eq!(Projection::from_id("nope"), None);
        assert_eq!(Projection::from_id(""), None);
        let mut idx: Vec<i32> = Projection::ALL.iter().map(|p| p.shader_index()).collect();
        idx.sort();
        assert_eq!(idx, vec![0, 1, 2, 3, 4]);
        assert!(!Projection::Globe.is_flat());
    }

    #[test]
    fn picker_options_match_the_enum() {
        // index.html lists the projections in order, with these labels.
        let html = include_str!("../index.html");
        let start = html.find("<div id=\"proj-menu\"").expect("picker");
        let end = start + html[start..].find("</div>").unwrap();
        let options: Vec<(&str, &str)> = html[start..end]
            .lines()
            .filter_map(|l| {
                let l = l.trim().strip_prefix("<button data-proj=\"")?;
                let (id, rest) = l.split_once("\" aria-pressed=")?;
                let label = rest.split_once('>')?.1.strip_suffix("</button>")?;
                Some((id, label))
            })
            .collect();
        let expected: Vec<(&str, &str)> = Projection::ALL
            .iter()
            .map(|p| (p.id(), p.label()))
            .collect();
        assert_eq!(options, expected);
    }

    #[test]
    fn next_and_prev_cycle() {
        let mut p = Projection::Globe;
        for _ in 0..Projection::ALL.len() {
            p = p.next();
        }
        assert_eq!(p, Projection::Globe);
        for q in Projection::ALL {
            assert_eq!(q.next().prev(), q);
        }
    }

    #[test]
    fn forward_inverse_round_trip() {
        for p in FLATS {
            let limit = if p == Projection::Mercator {
                85.0
            } else {
                90.0
            };
            let mut lat = -limit;
            while lat <= limit {
                for lon in [
                    -180.0, -179.9, -90.0, -1.0, 0.0, 0.5, 45.0, 120.0, 179.9, 180.0,
                ] {
                    let (x, y) = forward(p, deg(lat), deg(lon)).unwrap();
                    let (la, lo) = inverse(p, x, y).unwrap_or_else(|| {
                        panic!("{p:?} lat {lat} lon {lon} -> ({x}, {y}) not invertible")
                    });
                    assert!((la - deg(lat)).abs() < 1e-6, "{p:?} lat {lat}: {la}");
                    // The pole is a single point: longitude carries no information.
                    if lat.abs() < 89.99 || p == Projection::Equirectangular {
                        // Robinson and Mollweide are exact to float noise.
                        assert!(
                            (lo - deg(lon)).abs() < 1e-5,
                            "{p:?} lon {lon} lat {lat}: {lo}"
                        );
                    }
                }
                lat += 2.5;
            }
        }
    }

    #[test]
    fn inverse_forward_round_trip_on_the_plane() {
        for p in FLATS {
            let ym = p.y_max();
            for i in -9..=9 {
                for j in -9..=9 {
                    let y = ym * (i as f64 / 10.0);
                    let x = p.half_width() * (j as f64 / 10.0);
                    let Some((lat, lon)) = inverse(p, x, y) else {
                        continue;
                    };
                    let (x2, y2) = forward(p, lat, lon).unwrap();
                    assert!(
                        (x - x2).abs() < 1e-6 && (y - y2).abs() < 1e-6,
                        "{p:?} ({x},{y})"
                    );
                }
            }
        }
    }

    #[test]
    fn central_point_and_equator() {
        for p in FLATS {
            let (x, y) = forward(p, 0.0, 0.0).unwrap();
            assert!(x.abs() < 1e-12 && y.abs() < 1e-12, "{p:?}");
            assert_eq!(inverse(p, 0.0, 0.0), Some((0.0, 0.0)));
        }
        // Equator scales.
        assert!((forward(Projection::Robinson, 0.0, 1.0).unwrap().0 - 0.8487).abs() < 1e-12);
        assert!((forward(Projection::Mollweide, 0.0, 1.0).unwrap().0 - MOLLWEIDE_X).abs() < 1e-9);
    }

    #[test]
    fn known_values() {
        // Mercator at 45 degrees: ln(tan(67.5 deg)) = 0.881373587.
        let (_, y) = forward(Projection::Mercator, deg(45.0), 0.0).unwrap();
        assert!((y - 0.881_373_587_019_543).abs() < 1e-9);
        // Mollweide at 45 degrees: theta = 0.5 solves 2t + sin 2t = pi sin 45.
        let th = mollweide_theta(deg(45.0));
        assert!((2.0 * th + (2.0 * th).sin() - PI * deg(45.0).sin()).abs() < 1e-12);
        // Robinson at 45 degrees is an exact table row (index 9).
        let (x, y) = forward(Projection::Robinson, deg(45.0), PI).unwrap();
        assert!((x - 0.8487 * 0.8962 * PI).abs() < 1e-9);
        assert!((y - 1.3523 * 0.5571).abs() < 1e-9);
    }

    #[test]
    fn mercator_is_clamped_at_the_limit() {
        let p = Projection::Mercator;
        assert!((p.max_lat().to_degrees() - 85.0511).abs() < 1e-4);
        // y at the limit is pi: the map is a square.
        assert!((y_of_lat(p, MERCATOR_MAX_LAT) - PI).abs() < 1e-12);
        // Beyond the limit forward clamps; it never returns infinity.
        for lat in [86.0, 89.0, 90.0, -90.0, -86.0] {
            let (_, y) = forward(p, deg(lat), 0.0).unwrap();
            assert!(y.is_finite() && (y.abs() - PI).abs() < 1e-12, "{lat}: {y}");
        }
        assert!(in_domain(p, deg(85.0)));
        assert!(!in_domain(p, deg(86.0)));
        assert!(!in_domain(Projection::Equirectangular, deg(91.0)));
        assert!(in_domain(Projection::Equirectangular, deg(90.0)));
        // Past the map's edge there is nothing to pick.
        assert!(inverse(p, 0.0, PI + 0.01).is_none());
        assert!(inverse(p, 0.0, -PI - 0.01).is_none());
        assert!(inverse(p, 0.0, PI - 0.01).is_some());
        // The lat <-> y helpers clamp onto the map.
        assert!((lat_of_y(p, 100.0) - MERCATOR_MAX_LAT).abs() < 1e-9);
        assert!((lat_of_y(p, -100.0) + MERCATOR_MAX_LAT).abs() < 1e-9);
    }

    #[test]
    fn poles() {
        let up = Projection::Equirectangular;
        assert_eq!(forward(up, FRAC_PI_2, 1.0), Some((1.0, FRAC_PI_2)));
        // Mollweide's pole is a point; Robinson's is a line 0.5322 of the equator.
        let (x, y) = forward(Projection::Mollweide, FRAC_PI_2, 2.0).unwrap();
        assert!(x.abs() < 1e-6 && (y - 2f64.sqrt()).abs() < 1e-9);
        let (x, y) = forward(Projection::Robinson, FRAC_PI_2, PI).unwrap();
        assert!((x - 0.8487 * 0.5322 * PI).abs() < 1e-9 && (y - 1.3523).abs() < 1e-12);
        let (x, y) = forward(Projection::Robinson, -FRAC_PI_2, PI).unwrap();
        assert!(x > 0.0 && (y + 1.3523).abs() < 1e-12);
        // Picking the exact pole of Mollweide gives latitude 90.
        let (lat, _) = inverse(Projection::Mollweide, 0.0, 2f64.sqrt()).unwrap();
        assert!((lat - FRAC_PI_2).abs() < 1e-9);
        assert!(inverse(Projection::Mollweide, 0.3, 2f64.sqrt()).is_none());
        // Latitudes past the poles are clamped, not mirrored.
        assert_eq!(
            forward(Projection::Robinson, deg(95.0), 0.0),
            forward(Projection::Robinson, deg(90.0), 0.0)
        );
    }

    #[test]
    fn seam() {
        // One world: the edges at +-180 are on the outline, beyond is outside.
        for p in [Projection::Robinson, Projection::Mollweide] {
            let (x, y) = forward(p, deg(30.0), PI).unwrap();
            let (_, lon) = inverse(p, x, y).unwrap();
            assert!((lon - PI).abs() < 1e-6);
            assert!(inverse(p, x * 1.01, y).is_none(), "{p:?}");
            assert!(inverse(p, -x * 1.01, y).is_none(), "{p:?}");
            assert!(!p.repeats());
        }
        // Cylindrical maps repeat: longitudes beyond 180 are the next world.
        for p in [Projection::Equirectangular, Projection::Mercator] {
            let (lat, lon) = inverse(p, 3.0 * PI, 0.2).unwrap();
            assert!(lat.is_finite() && (lon - 3.0 * PI).abs() < 1e-12);
            assert!(p.repeats());
        }
    }

    #[test]
    fn non_finite_input_is_rejected() {
        for p in Projection::ALL {
            for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                assert!(forward(p, bad, 0.0).is_none(), "{p:?}");
                assert!(forward(p, 0.0, bad).is_none(), "{p:?}");
                assert!(inverse(p, bad, 0.0).is_none(), "{p:?}");
                assert!(inverse(p, 0.0, bad).is_none(), "{p:?}");
            }
        }
        assert!(forward(Projection::Globe, 0.0, 0.0).is_none());
        assert!(inverse(Projection::Globe, 0.0, 0.0).is_none());
        for p in FLATS {
            assert!(lat_of_y(p, f64::NAN).is_finite());
        }
    }

    #[test]
    fn y_is_monotonic_in_latitude() {
        for p in FLATS {
            let mut prev = f64::NEG_INFINITY;
            let mut lat = -90.0;
            while lat <= 90.0 {
                let y = y_of_lat(p, deg(lat));
                assert!(y >= prev - 1e-12, "{p:?} at {lat}");
                prev = y;
                lat += 0.5;
            }
        }
    }

    #[test]
    fn equal_area_projection_preserves_area_ratios() {
        // Mollweide is equal-area: the map area between two parallels equals
        // 2 pi (sin b - sin a) times the same constant at any longitude span.
        // Integrate numerically the area between 0 and `lat` degrees.
        let area = |lat: f64| {
            let n = 4000;
            let mut sum = 0.0;
            for i in 0..n {
                let l = lat * (i as f64 + 0.5) / n as f64;
                let (x, _) = forward(Projection::Mollweide, l, PI).unwrap();
                let (_, y0) =
                    forward(Projection::Mollweide, lat * i as f64 / n as f64, 0.0).unwrap();
                let (_, y1) =
                    forward(Projection::Mollweide, lat * (i + 1) as f64 / n as f64, 0.0).unwrap();
                sum += 2.0 * x * (y1 - y0);
            }
            sum
        };
        // The whole ellipse has area 4 pi, the sphere's, so the ratio is 1.
        let k = area(deg(30.0)) / (2.0 * PI * deg(30.0).sin());
        let k2 = area(deg(75.0)) / (2.0 * PI * deg(75.0).sin());
        assert!((k - k2).abs() / k < 1e-3, "{k} vs {k2}");
        assert!((k - 1.0).abs() < 1e-3, "{k}"); // the map has the sphere's area
    }

    #[test]
    fn x_scale_matches_numeric_derivative() {
        for p in FLATS {
            for lat in [-80.0, -45.0, 0.0, 30.0, 70.0] {
                let (x, _) = forward(p, deg(lat), 0.01).unwrap();
                let (x0, _) = forward(p, deg(lat), 0.0).unwrap();
                assert!(
                    ((x - x0) / 0.01 - x_scale(p, deg(lat))).abs() < 1e-9,
                    "{p:?}"
                );
            }
        }
        // Mercator stretches parallels by 1/cos(lat).
        let s = parallel_stretch(Projection::Mercator, deg(60.0));
        assert!((s - 2.0).abs() < 1e-9);
        assert!(parallel_stretch(Projection::Mercator, FRAC_PI_2).is_finite());
        assert!(parallel_stretch(Projection::Mollweide, FRAC_PI_2) > 0.0);
    }

    #[test]
    fn line_copies() {
        // A view narrower than one world needs only the central copy.
        assert_eq!(line_wraps(Projection::Mercator, 2.0), (0, false));
        assert_eq!(line_wraps(Projection::Equirectangular, PI), (0, false));
        // Past 180 degrees either side, the next copy is drawn, and so on.
        assert_eq!(line_wraps(Projection::Mercator, PI + 0.1), (1, false));
        assert_eq!(line_wraps(Projection::Mercator, 3.0 * PI + 0.1), (2, false));
        // One world with an outline: neighbours catch seam-crossing segments
        // and every copy is clipped to the outline.
        assert_eq!(line_wraps(Projection::Robinson, 1.0), (1, true));
        assert_eq!(line_wraps(Projection::Mollweide, 50.0), (1, true));
        // Bad input never explodes.
        assert_eq!(line_wraps(Projection::Mercator, f64::NAN), (0, false));
        assert_eq!(line_wraps(Projection::Mercator, f64::INFINITY).0, 4);
    }

    #[test]
    fn morph_timeline() {
        let mut m = Morph::new(Projection::Globe);
        assert!(!m.animating());
        assert_eq!(m.blend(), Blend::settled(Projection::Globe));
        m.start(Projection::Mercator, false);
        assert!(m.animating());
        let b = m.blend();
        assert_eq!(
            (b.a, b.b, b.t),
            (Projection::Globe, Projection::Mercator, 0.0)
        );
        assert!(
            b.is_settled() && b.shown() == Projection::Globe,
            "t = 0 is the globe"
        );
        assert!(m.tick(MORPH_MS / 2.0));
        let b = m.blend();
        assert!(!b.is_settled() && (b.t - 0.5).abs() < 1e-12);
        assert!((b.globe_weight() - 0.5).abs() < 1e-12);
        assert!(m.tick(MORPH_MS));
        assert!(!m.animating());
        assert_eq!(m.blend(), Blend::settled(Projection::Mercator));
        assert!(!m.tick(16.0), "nothing to do once settled");
        // And back.
        m.start(Projection::Globe, false);
        m.tick(MORPH_MS / 4.0);
        assert!(m.blend().globe_weight() > 0.0 && m.blend().globe_weight() < 0.5);
    }

    #[test]
    fn reduced_motion_skips_the_animation() {
        let mut m = Morph::new(Projection::Globe);
        m.start(Projection::Robinson, true);
        assert!(!m.animating());
        assert_eq!(m.blend(), Blend::settled(Projection::Robinson));
        // Choosing the current target is a no-op.
        m.start(Projection::Robinson, false);
        assert!(!m.animating());
    }

    #[test]
    fn retargeting_mid_morph_is_continuous_enough() {
        let mut m = Morph::new(Projection::Globe);
        m.start(Projection::Mercator, false);
        m.tick(MORPH_MS * 0.8);
        m.start(Projection::Mollweide, false);
        let b = m.blend();
        assert_eq!((b.a, b.b), (Projection::Mercator, Projection::Mollweide));
        m.tick(f64::NAN);
        m.tick(-5.0);
        assert!(m.blend().t >= 0.0 && m.blend().t <= 1.0);
    }

    #[test]
    fn ease_is_clamped_and_monotonic() {
        assert_eq!(ease(-1.0), 0.0);
        assert_eq!(ease(2.0), 1.0);
        assert!((ease(0.5) - 0.5).abs() < 1e-12);
        let mut prev = 0.0;
        for i in 0..=100 {
            let e = ease(i as f64 / 100.0);
            assert!(e >= prev);
            prev = e;
        }
    }

    // ---- GLSL mirror checks ---------------------------------------------
    //
    // The shaders cannot run natively, so these tests keep them honest: every
    // constant and table is parsed out of the GLSL source and compared with
    // the Rust values above, the shared GLSL functions must be textually
    // identical across files, and a Rust port of the GLSL algorithm in f32
    // (same iteration counts) must agree with the f64 reference.

    const FRAG: &str = include_str!("shaders/globe.frag");
    const MORPH_VERT: &str = include_str!("shaders/morph.vert");
    const LINE_VERT: &str = include_str!("shaders/line.vert");

    /// Numbers inside the braces of `const float NAME[19] = float[19](...)`.
    fn glsl_table(src: &str, name: &str) -> Vec<f64> {
        let start = src
            .find(&format!("const float {name}[19]"))
            .unwrap_or_else(|| panic!("{name} missing"));
        let open = start + src[start..].find("float[19](").unwrap() + "float[19](".len();
        let close = open + src[open..].find(')').unwrap();
        src[open..close]
            .split(',')
            .map(|v| v.trim().parse::<f64>().unwrap())
            .collect()
    }

    /// Value of `const float NAME = <number>;`.
    fn glsl_const(src: &str, name: &str) -> f64 {
        let start = src
            .find(&format!("const float {name} = "))
            .unwrap_or_else(|| panic!("{name} missing"));
        let from = start + format!("const float {name} = ").len();
        let end = from + src[from..].find(';').unwrap();
        src[from..end].trim().parse::<f64>().unwrap()
    }

    fn glsl_fn<'a>(src: &'a str, signature: &str) -> &'a str {
        let start = src.find(signature).expect("function present");
        let end = start + src[start..].find("\n}").expect("function end") + 2;
        &src[start..end]
    }

    #[test]
    fn glsl_tables_and_constants_match_rust() {
        for (name, src) in [
            ("globe.frag", FRAG),
            ("morph.vert", MORPH_VERT),
            ("line.vert", LINE_VERT),
        ] {
            assert_eq!(glsl_table(src, "ROB_X"), ROBINSON_PLEN, "{name}");
            assert_eq!(glsl_table(src, "ROB_Y"), ROBINSON_PDFE, "{name}");
            for (c, v) in [
                ("MERC_MAX", MERCATOR_MAX_LAT),
                ("ROB_XS", ROBINSON_X_SCALE),
                ("ROB_YS", ROBINSON_Y_SCALE),
                ("MOLL_X", MOLLWEIDE_X),
            ] {
                assert!((glsl_const(src, c) - v).abs() < 1e-12, "{name} {c}");
            }
            if name != "globe.frag" {
                // The fragment shader inverts Mollweide in closed form.
                assert!(
                    src.contains(&format!("i < {MOLLWEIDE_ITERATIONS};")),
                    "{name}: Mollweide iteration count"
                );
            }
        }
    }

    #[test]
    fn forward_projection_glsl_is_identical_in_both_vertex_shaders() {
        for sig in [
            "float mollTheta(float lat)",
            "vec2 flatForward(int id, float lat, float lon)",
        ] {
            assert_eq!(glsl_fn(MORPH_VERT, sig), glsl_fn(LINE_VERT, sig), "{sig}");
        }
    }

    #[test]
    fn projection_ids_in_glsl_match_the_enum() {
        // flatForward and flatInverse dispatch on these integers.
        for (p, body) in [
            (Projection::Equirectangular, "if (id == 1)"),
            (Projection::Mercator, "if (id == 2)"),
            (Projection::Robinson, "if (id == 3)"),
        ] {
            assert!(MORPH_VERT.contains(body) && FRAG.contains(body), "{p:?}");
            assert_eq!(
                p.shader_index(),
                body.trim_start_matches("if (id == ")
                    .trim_end_matches(')')
                    .parse::<i32>()
                    .unwrap()
            );
        }
        // Mollweide is the fall-through branch (id 4).
        assert_eq!(Projection::Mollweide.shader_index(), 4);
    }

    /// Rust port of the GLSL `mollTheta`, in f32 with the shader's loop.
    fn moll_theta_glsl(lat: f32) -> f32 {
        let pi = std::f32::consts::PI;
        let mut th = lat;
        for _ in 0..MOLLWEIDE_ITERATIONS {
            let d = 2.0 + 2.0 * (2.0 * th).cos();
            if d.abs() < 1e-5 {
                break;
            }
            th -= (2.0 * th + (2.0 * th).sin() - pi * lat.sin()) / d;
        }
        th
    }

    #[test]
    fn place_function_is_identical_in_both_vertex_shaders() {
        let sig = "vec2 place(int id, float cy, float lat, float rel, out float front)";
        assert_eq!(glsl_fn(MORPH_VERT, sig), glsl_fn(LINE_VERT, sig));
    }

    #[test]
    fn glsl_mollweide_iteration_matches_the_reference() {
        let mut lat = -90.0f64;
        while lat <= 90.0 {
            let l = deg(lat);
            let th = moll_theta_glsl(l as f32) as f64;
            let exact = mollweide_theta(l);
            // Compare where it matters: the projected position.
            let (xg, yg) = (MOLLWEIDE_X * PI * th.cos(), 2f64.sqrt() * th.sin());
            let (xe, ye) = (MOLLWEIDE_X * PI * exact.cos(), 2f64.sqrt() * exact.sin());
            assert!(
                (xg - xe).abs() < 2e-3 && (yg - ye).abs() < 2e-3,
                "lat {lat}: ({xg},{yg}) vs ({xe},{ye})"
            );
            lat += 0.25;
        }
    }
}
