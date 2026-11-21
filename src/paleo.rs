//! Deep time: coastlines reconstructed back to 1000 million years with the
//! Merdith et al. (2021) plate model. Browser-free.
//!
//! The model is a set of finite rotations (`paleo_rotations.tsv`, written by
//! `scripts/convert_merdith.py`): at listed ages, the rotation of a moving
//! plate relative to a fixed one, as an Euler pole and an angle. Between
//! listed ages the rotation is interpolated (spherical linear interpolation
//! of the quaternions, as GPlates does), and a plate's rotation relative to
//! the absolute frame (plate 0) is the product of the rotations along its
//! chain of fixed plates. The coastlines (`paleo_coastlines.bin`, HIST in
//! tenths of Ma) are present-day polygons tagged with their plate; a
//! polygon is drawn while the age is inside its interval, rotated with its
//! plate.
//!
//! Tests compare reconstructed points with the GPlates Web Service.

use crate::geo::Shape;
use crate::history::Timeline;
use std::collections::HashMap;

/// Oldest age the model covers, Ma.
pub const MAX_MA: f64 = 1000.0;

/// A unit quaternion (w, x, y, z) for rotations of unit vectors with x
/// towards (0 N, 0 E), y towards (0 N, 90 E) and z the north pole.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat(pub [f64; 4]);

impl Quat {
    pub const ID: Quat = Quat([1.0, 0.0, 0.0, 0.0]);

    /// The rotation by `angle` degrees about the pole at `lat`, `lon`.
    pub fn from_pole(lat: f64, lon: f64, angle: f64) -> Quat {
        let a = unit(lat, lon);
        let (s, c) = (angle.to_radians() / 2.0).sin_cos();
        Quat([c, s * a[0], s * a[1], s * a[2]])
    }

    pub fn mul(self, o: Quat) -> Quat {
        let [a1, b1, c1, d1] = self.0;
        let [a2, b2, c2, d2] = o.0;
        Quat([
            a1 * a2 - b1 * b2 - c1 * c2 - d1 * d2,
            a1 * b2 + b1 * a2 + c1 * d2 - d1 * c2,
            a1 * c2 - b1 * d2 + c1 * a2 + d1 * b2,
            a1 * d2 + b1 * c2 - c1 * b2 + d1 * a2,
        ])
    }

    pub fn rotate(self, v: [f64; 3]) -> [f64; 3] {
        let [w, x, y, z] = self.0;
        // v + 2 q x (q x v + w v), with q the vector part.
        let t = [
            2.0 * (y * v[2] - z * v[1]),
            2.0 * (z * v[0] - x * v[2]),
            2.0 * (x * v[1] - y * v[0]),
        ];
        [
            v[0] + w * t[0] + (y * t[2] - z * t[1]),
            v[1] + w * t[1] + (z * t[0] - x * t[2]),
            v[2] + w * t[2] + (x * t[1] - y * t[0]),
        ]
    }

    /// Spherical interpolation from `self` (f = 0) to `o` (f = 1), the short
    /// way round.
    pub fn slerp(self, o: Quat, f: f64) -> Quat {
        let mut b = o.0;
        let mut d: f64 = (0..4).map(|i| self.0[i] * b[i]).sum();
        if d < 0.0 {
            b = b.map(|v| -v);
            d = -d;
        }
        if d > 0.9995 {
            let q: [f64; 4] = std::array::from_fn(|i| self.0[i] + f * (b[i] - self.0[i]));
            let n = q.iter().map(|v| v * v).sum::<f64>().sqrt();
            return Quat(q.map(|v| v / n));
        }
        let th = d.clamp(-1.0, 1.0).acos();
        let (s0, s1) = (((1.0 - f) * th).sin(), (f * th).sin());
        let st = th.sin();
        Quat(std::array::from_fn(|i| (s0 * self.0[i] + s1 * b[i]) / st))
    }
}

/// Unit vector of a lat/lon in degrees (see [`Quat`]).
pub fn unit(lat: f64, lon: f64) -> [f64; 3] {
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()]
}

/// Lat/lon in degrees of a unit vector.
pub fn lat_lon(v: [f64; 3]) -> (f64, f64) {
    let lat = v[2].clamp(-1.0, 1.0).asin().to_degrees();
    (lat, v[1].atan2(v[0]).to_degrees())
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pole {
    age: f64,
    rot: Quat,
    fixed: u32,
}

/// The rotation model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rotations {
    /// Per moving plate, its listed rotations in file order.
    plates: HashMap<u32, Vec<Pole>>,
}

/// Parse `paleo_rotations.tsv`. Malformed lines are skipped; a wrong header
/// is an error.
pub fn parse_rotations(text: &str) -> Result<Rotations, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("MAPSROT1") {
        return Err("not a rotations file (bad header)".into());
    }
    let mut plates: HashMap<u32, Vec<Pole>> = HashMap::new();
    for line in lines {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 6 {
            continue;
        }
        let (Ok(moving), Ok(age), Ok(lat), Ok(lon), Ok(angle), Ok(fixed)) = (
            f[0].parse::<u32>(),
            f[1].parse::<f64>(),
            f[2].parse::<f64>(),
            f[3].parse::<f64>(),
            f[4].parse::<f64>(),
            f[5].parse::<u32>(),
        ) else {
            continue;
        };
        if ![age, lat, lon, angle].iter().all(|v| v.is_finite()) {
            continue;
        }
        plates.entry(moving).or_default().push(Pole {
            age,
            rot: Quat::from_pole(lat, lon, angle),
            fixed,
        });
    }
    Ok(Rotations { plates })
}

impl Rotations {
    /// Rotation of `plate` relative to its fixed plate at `age`, and that
    /// fixed plate: interpolated between the two listed ages around `age`
    /// that share a fixed plate.
    fn relative(&self, plate: u32, age: f64) -> Option<(Quat, u32)> {
        let poles = self.plates.get(&plate)?;
        for w in poles.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a.fixed != b.fixed || !(a.age <= age && age <= b.age) {
                continue;
            }
            let span = b.age - a.age;
            let f = if span > 0.0 {
                (age - a.age) / span
            } else {
                0.0
            };
            return Some((a.rot.slerp(b.rot, f), a.fixed));
        }
        // A single listed age that is exactly the one asked for.
        poles
            .iter()
            .find(|p| p.age == age)
            .map(|p| (p.rot, p.fixed))
    }

    /// Rotation of `plate` relative to the absolute frame (plate 0) at
    /// `age` Ma, or `None` where the model has no rotation for it then.
    pub fn total(&self, plate: u32, age: f64) -> Option<Quat> {
        let mut q = Quat::ID;
        let mut p = plate;
        // Chains are short; the bound stops a malformed loop.
        for _ in 0..64 {
            if p == 0 {
                return Some(q);
            }
            let (r, fixed) = self.relative(p, age)?;
            q = r.mul(q);
            p = fixed;
        }
        None
    }

    /// Where a present-day point on `plate` was at `age` (lat, lon degrees).
    #[cfg(test)]
    pub fn reconstruct(&self, plate: u32, age: f64, lat: f64, lon: f64) -> Option<(f64, f64)> {
        Some(lat_lon(self.total(plate, age)?.rotate(unit(lat, lon))))
    }
}

/// A reconstructed polygon: its rings (lon, lat degrees, as `Shape`s) and
/// its continent group for coloring (the first digit of the plate id:
/// 1 North America, 2 South America, 3 Europe, 4 northern Asia, 5 India and
/// the Indian Ocean, 6 Asia, 7 Africa, 8 Australia and Antarctica, 9 the
/// Pacific).
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub rings: Vec<Shape>,
    pub group: u8,
}

/// Political palette index (`PALETTE` in `globe.frag`) of each continent
/// group, by hand so that neighbors in Pangaea and Gondwana differ: North
/// America clay, South America straw, Europe sage, northern Asia and Africa
/// slate blue (never neighbors), India apricot, Asia heather, Australia and
/// Antarctica clay, the Pacific sage.
pub const GROUP_COLORS: [u8; 10] = [0, 0, 1, 2, 3, 5, 4, 3, 0, 2];

/// The first digit of a plate id (0 for plate 0).
pub fn group_of(plate: u32) -> u8 {
    let mut p = plate;
    while p >= 10 {
        p /= 10;
    }
    p as u8
}

/// The coastline polygons that exist at `age` Ma, rotated to where they
/// were then.
pub fn reconstruct(coasts: &Timeline, rot: &Rotations, age: f64) -> Vec<Piece> {
    let t = -(age * 10.0).round() as i64;
    let mut cache: HashMap<u32, Option<Quat>> = HashMap::new();
    coasts
        .holding(t)
        .into_iter()
        .filter_map(|ri| {
            let r = &coasts.records[ri];
            let q = (*cache.entry(r.tag).or_insert_with(|| rot.total(r.tag, age)))?;
            let rings = coasts
                .rings_of(r)
                .map(|s| Shape {
                    points: s
                        .points
                        .iter()
                        .map(|p| {
                            let (lat, lon) = lat_lon(q.rotate(unit(p[1], p[0])));
                            [lon, lat]
                        })
                        .collect(),
                    closed: true,
                })
                .collect();
            Some(Piece {
                rings,
                group: group_of(r.tag),
            })
        })
        .collect()
}

/// Rasterize pieces into a `width x width/2` equirectangular grid of group
/// ids (0 for sea), row 0 at the north. Rotated rings may cross the ±180
/// meridian or go round a pole: each ring's longitudes are unwrapped into a
/// continuous run, a ring that winds once round the globe is closed over
/// the nearer pole, and spans wrap round the raster's columns.
pub fn rasterize(pieces: &[Piece], width: usize) -> Vec<u8> {
    let height = width / 2;
    let mut out = vec![0u8; width * height];
    if height == 0 {
        return out;
    }
    let (dx, dy) = (360.0 / width as f64, 180.0 / height as f64);
    let mut xs: Vec<f64> = Vec::new();
    for piece in pieces {
        let rings: Vec<Vec<[f64; 2]>> =
            piece.rings.iter().map(|r| unwrap_ring(&r.points)).collect();
        let (lo, hi) = rings
            .iter()
            .flatten()
            .fold((90.0f64, -90.0f64), |(lo, hi), p| {
                (lo.min(p[1]), hi.max(p[1]))
            });
        let r0 = ((90.0 - hi) / dy - 0.5).ceil().max(0.0) as usize;
        let r1 = ((90.0 - lo) / dy - 0.5).floor();
        if r1 < 0.0 {
            continue;
        }
        for row in r0..=(r1 as usize).min(height - 1) {
            let lat = 90.0 - (row as f64 + 0.5) * dy;
            xs.clear();
            for p in &rings {
                let mut j = p.len() - 1;
                for i in 0..p.len() {
                    let (a, b) = (p[i], p[j]);
                    if (a[1] > lat) != (b[1] > lat) {
                        xs.push(a[0] + (lat - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                    }
                    j = i;
                }
            }
            xs.sort_by(|a, b| a.total_cmp(b));
            for pair in xs.as_chunks::<2>().0 {
                let c0 = ((pair[0] + 180.0) / dx - 0.5).ceil() as i64;
                let c1 = ((pair[1] + 180.0) / dx - 0.5).ceil() as i64;
                // At most one full turn.
                let c1 = c1.min(c0 + width as i64);
                for c in c0..c1 {
                    let col = c.rem_euclid(width as i64) as usize;
                    out[row * width + col] = piece.group;
                }
            }
        }
    }
    out
}

/// A ring with continuous longitudes (each step under 180 degrees), closed
/// over a pole if it winds once round the globe.
fn unwrap_ring(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(points.len() + 2);
    for p in points {
        let lon = match out.last() {
            None => p[0],
            Some(prev) => prev[0] + wrap180(p[0] - prev[0]),
        };
        out.push([lon, p[1]]);
    }
    let (Some(first), Some(last)) = (out.first().copied(), out.last().copied()) else {
        return out;
    };
    let winding = last[0] + wrap180(first[0] - last[0]) - first[0];
    if winding.abs() > 180.0 {
        // Round a pole: the nearer one to the ring's mean latitude.
        let mean = out.iter().map(|p| p[1]).sum::<f64>() / out.len() as f64;
        let pole = if mean >= 0.0 { 90.0 } else { -90.0 };
        let end = first[0] + winding;
        out.push([end, last[1]]);
        out.push([end, pole]);
        out.push([first[0], pole]);
    }
    out
}

fn wrap180(d: f64) -> f64 {
    (d + 180.0).rem_euclid(360.0) - 180.0
}

/// An age for people: "250 million years ago", "the present".
pub fn format_ma(ma: f64) -> String {
    if ma <= 0.0 {
        "Present".into()
    } else {
        format!("{ma:.0} million years ago")
    }
}

/// The geological period of an age (ICS 2023 boundaries, Ma).
pub fn period(ma: f64) -> &'static str {
    const PERIODS: [(f64, &str); 16] = [
        (2.58, "Quaternary"),
        (23.03, "Neogene"),
        (66.0, "Paleogene"),
        (145.0, "Cretaceous"),
        (201.4, "Jurassic"),
        (251.9, "Triassic"),
        (298.9, "Permian"),
        (358.9, "Carboniferous"),
        (419.2, "Devonian"),
        (443.8, "Silurian"),
        (485.4, "Ordovician"),
        (538.8, "Cambrian"),
        (635.0, "Ediacaran"),
        (720.0, "Cryogenian"),
        (1000.0, "Tonian"),
        (f64::INFINITY, "Stenian"),
    ];
    PERIODS
        .iter()
        .find(|(end, _)| ma <= *end)
        .map_or("Stenian", |(_, n)| n)
}

/// `&ma=<age>` in a URL hash, clamped to the model's span.
pub fn parse_hash_ma(hash: &str) -> Option<f64> {
    let v = hash
        .trim_start_matches('#')
        .split('&')
        .skip(1)
        .find_map(|kv| kv.strip_prefix("ma="))?;
    v.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .map(|v| v.round().clamp(0.0, MAX_MA))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Rotations {
        parse_rotations(include_str!("../assets/data/paleo_rotations.tsv")).unwrap()
    }

    /// GPlates Web Service, model MERDITH2021: reconstruct_points with the
    /// plate id given (lon, lat, plate, age -> lon, lat), fetched 2026-11-21.
    const GWS: [(f64, f64, u32, f64, [f64; 2]); 30] = [
        (-60.0, -10.0, 201, 50.0, [-44.54, -16.8928]),
        (-60.0, -10.0, 201, 100.0, [-25.241, -11.4501]),
        (-60.0, -10.0, 201, 250.0, [-24.1257, -21.3863]),
        (-60.0, -10.0, 201, 500.0, [-138.1529, -48.7755]),
        (-60.0, -10.0, 201, 800.0, [119.7592, -47.732]),
        (20.0, 0.0, 701, 50.0, [19.4964, -14.7798]),
        (20.0, 0.0, 701, 100.0, [17.4837, -15.2035]),
        (20.0, 0.0, 701, 250.0, [8.9891, -32.625]),
        (20.0, 0.0, 701, 500.0, [169.8196, -64.4717]),
        (20.0, 0.0, 701, 800.0, [15.708, -41.2167]),
        (-100.0, 40.0, 101, 50.0, [-72.6667, 42.2501]),
        (-100.0, 40.0, 101, 100.0, [-51.9611, 43.2872]),
        (-100.0, 40.0, 101, 250.0, [-25.566, 14.2772]),
        (-100.0, 40.0, 101, 500.0, [-148.3734, 2.0046]),
        (-100.0, 40.0, 101, 800.0, [106.3339, -20.3544]),
        (10.0, 50.0, 301, 50.0, [12.4413, 39.8387]),
        (10.0, 50.0, 301, 100.0, [25.5342, 42.2079]),
        (10.0, 50.0, 301, 250.0, [29.8693, 21.8796]),
        (10.0, 50.0, 301, 500.0, [-104.4815, -29.2667]),
        (10.0, 50.0, 301, 800.0, [163.786, -30.5331]),
        (135.0, -25.0, 801, 50.0, [140.3721, -45.4497]),
        (135.0, -25.0, 801, 100.0, [135.4076, -58.415]),
        (135.0, -25.0, 801, 250.0, [119.6746, -60.4121]),
        (135.0, -25.0, 801, 500.0, [126.5682, -0.5119]),
        (135.0, -25.0, 801, 800.0, [112.7675, 10.6559]),
        (78.0, 20.0, 501, 50.0, [72.2919, -5.3174]),
        (78.0, 20.0, 501, 100.0, [53.6813, -37.2809]),
        (78.0, 20.0, 501, 250.0, [48.4683, -49.0516]),
        (78.0, 20.0, 501, 500.0, [127.8321, -40.938]),
        (78.0, 20.0, 501, 800.0, [-12.6445, 65.7441]),
    ];

    fn angle_deg(a: (f64, f64), b: (f64, f64)) -> f64 {
        let (u, v) = (unit(a.0, a.1), unit(b.0, b.1));
        let d: f64 = (0..3).map(|i| u[i] * v[i]).sum();
        d.clamp(-1.0, 1.0).acos().to_degrees()
    }

    #[test]
    fn quaternions_rotate_like_their_pole() {
        // 90 degrees about the north pole takes 0E to 90E.
        let q = Quat::from_pole(90.0, 0.0, 90.0);
        let (lat, lon) = lat_lon(q.rotate(unit(0.0, 0.0)));
        assert!(lat.abs() < 1e-12 && (lon - 90.0).abs() < 1e-12);
        // Composition and slerp halfway.
        let qq = q.mul(q);
        let (_, lon2) = lat_lon(qq.rotate(unit(0.0, 0.0)));
        assert!((lon2 - 180.0).abs() < 1e-9);
        let half = Quat::ID.slerp(qq, 0.5);
        let (_, lon3) = lat_lon(half.rotate(unit(0.0, 0.0)));
        assert!((lon3 - 90.0).abs() < 1e-9);
    }

    #[test]
    fn reconstructions_match_gplates() {
        let m = model();
        let mut worst: f64 = 0.0;
        for (lon, lat, plate, age, want) in GWS {
            let (rlat, rlon) = m.reconstruct(plate, age, lat, lon).unwrap();
            let err = angle_deg((rlat, rlon), (want[1], want[0]));
            worst = worst.max(err);
            assert!(
                err < 0.05,
                "plate {plate} at {age} Ma: {rlat},{rlon} vs {want:?} ({err})"
            );
        }
        assert!(worst < 0.05, "{worst}");
        // The present is the present.
        let (lat, lon) = m.reconstruct(201, 0.0, -10.0, -60.0).unwrap();
        assert!((lat + 10.0).abs() < 1e-6 && (lon + 60.0).abs() < 1e-6);
        assert!(m.total(123_456, 10.0).is_none(), "unknown plate");
    }

    #[test]
    fn coastlines_reconstruct_and_fill() {
        let m = model();
        let coasts =
            crate::history::decode(include_bytes!("../assets/data/paleo_coastlines.bin")).unwrap();
        // Today the raster has land where land is.
        let pieces = reconstruct(&coasts, &m, 0.0);
        assert!(pieces.len() > 500, "{}", pieces.len());
        let w = 720;
        let r = rasterize(&pieces, w);
        let at = |lat: f64, lon: f64| {
            let c = (((lon + 180.0) / 360.0 * w as f64) as usize).min(w - 1);
            let row = (((90.0 - lat) / 180.0 * (w / 2) as f64) as usize).min(w / 2 - 1);
            r[row * w + c]
        };
        assert_eq!(at(-10.0, -60.0), 2, "Amazonia, South America");
        assert_eq!(at(0.0, 20.0), 7, "Congo, Africa");
        assert_eq!(at(0.0, -30.0), 0, "the Atlantic");
        assert_eq!(at(-85.0, 0.0), 8, "Antarctica round the pole");
        let land = r.iter().filter(|&&v| v > 0).count() as f64 / r.len() as f64;
        assert!((0.25..0.45).contains(&land), "{land}");
        // 250 Ma: Pangaea. The Atlantic is closed: Africa's west coast and
        // South America's east coast are together.
        let p250 = reconstruct(&coasts, &m, 250.0);
        let r250 = rasterize(&p250, w);
        assert!(r250.iter().filter(|&&v| v > 0).count() > r.len() / 5);
        let (lat, lon) = m.reconstruct(701, 250.0, 5.0, 9.0).unwrap(); // Gabon coast
        let (lat2, lon2) = m.reconstruct(201, 250.0, -5.0, -35.0).unwrap(); // Brazil's tip
        assert!(angle_deg((lat, lon), (lat2, lon2)) < 15.0);
    }

    #[test]
    fn rings_over_the_seam_and_poles_fill() {
        let ring = |pts: &[[f64; 2]]| Shape {
            points: pts.to_vec(),
            closed: true,
        };
        // A square across the antimeridian.
        let seam = Piece {
            rings: vec![ring(&[
                [170.0, -5.0],
                [-170.0, -5.0],
                [-170.0, 5.0],
                [170.0, 5.0],
            ])],
            group: 3,
        };
        // A cap round the south pole.
        let cap = Piece {
            rings: vec![ring(&[[0.0, -70.0], [120.0, -70.0], [-120.0, -70.0]])],
            group: 8,
        };
        let w = 360;
        let r = rasterize(&[seam, cap], w);
        let at = |lat: f64, lon: f64| {
            let c = (((lon + 180.0) / 360.0 * w as f64) as usize).min(w - 1);
            let row = (((90.0 - lat) / 180.0 * (w / 2) as f64) as usize).min(w / 2 - 1);
            r[row * w + c]
        };
        assert_eq!(at(0.0, 179.5), 3);
        assert_eq!(at(0.0, -179.5), 3);
        assert_eq!(at(0.0, 0.0), 0, "not the rest of the row");
        assert_eq!(at(-89.0, 45.0), 8);
        assert_eq!(at(-80.0, -100.0), 8);
        assert_eq!(at(-60.0, 0.0), 0);
    }

    #[test]
    fn ages_read_and_parse() {
        assert_eq!(format_ma(250.0), "250 million years ago");
        assert_eq!(format_ma(0.0), "Present");
        assert_eq!(period(250.0), "Triassic");
        assert_eq!(period(260.0), "Permian");
        assert_eq!(period(66.0), "Paleogene");
        assert_eq!(period(65.9), "Paleogene");
        assert_eq!(period(100.0), "Cretaceous");
        assert_eq!(period(1.0), "Quaternary");
        assert_eq!(period(999.0), "Tonian");
        assert_eq!(parse_hash_ma("#0,0,3&ma=250"), Some(250.0));
        assert_eq!(parse_hash_ma("#0,0,3&ma=5000"), Some(MAX_MA));
        assert_eq!(parse_hash_ma("#0,0,3&ma=-3"), Some(0.0));
        assert_eq!(parse_hash_ma("#0,0,3&ma=x"), None);
        assert_eq!(parse_hash_ma("#0,0,3"), None);
        assert_eq!(group_of(201), 2);
        assert_eq!(group_of(16101), 1);
        assert_eq!(group_of(0), 0);
        assert!(GROUP_COLORS
            .iter()
            .all(|&c| c < crate::country::PALETTE_SIZE));
        // Neighbors in Pangaea differ.
        for (a, b) in [(1, 7), (2, 7), (1, 3), (3, 7), (7, 8), (5, 7), (2, 8)] {
            assert_ne!(GROUP_COLORS[a], GROUP_COLORS[b], "groups {a} and {b}");
        }
    }
}
