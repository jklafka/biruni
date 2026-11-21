//! HEALPix geometry for HiPS sky surveys (nested scheme). Browser-free.
//!
//! The sphere is cut into 12 base faces; at order `k` each face is a
//! `2^k x 2^k` grid, and a nested index is `face * 4^k` plus the bits of
//! the face coordinates interleaved (x in the even bits, y in the odd).
//! `xyf_to_loc` and `loc_to_xyf` follow the HEALPix C++ library's
//! `xyf2loc` / `loc2xyf` (Gorski et al. 2005), with continuous face
//! fractions `x`, `y` in `[0, 1]`.
//!
//! Directions here are unit vectors in the sky view's equatorial space
//! (`sky.rs`: +Y the pole, RA 0h on +Z, 6h on +X). HiPS surveys in the
//! equatorial frame use ICRS, which agrees with J2000 to a few hundredths
//! of an arcsecond.
//!
//! In a HiPS tile image, column `c` and row `r` (from the top) cover the
//! face fractions `y = (ty + c / w) / 2^k` and `x = (tx + r / w) / 2^k`:
//! the image's columns run along the face's y axis. This was measured, not
//! assumed: on the DSS color Allsky atlas, bright HYG stars land on bright
//! pixels in this orientation and on sky background in the other seven.

use std::f64::consts::FRAC_PI_4;

const JRLL: [i32; 12] = [2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4];
const JPLL: [i32; 12] = [1, 3, 5, 7, 0, 2, 4, 6, 1, 3, 5, 7];

/// `(z, phi)` (cos of the colatitude, and right ascension in `[0, 2 pi)`)
/// of face fractions `x`, `y` on `face`.
pub fn xyf_to_loc(x: f64, y: f64, face: usize) -> (f64, f64) {
    let jr = JRLL[face] as f64 - x - y;
    let (nr, z) = if jr < 1.0 {
        (jr, 1.0 - jr * jr / 3.0)
    } else if jr > 3.0 {
        let nr = 4.0 - jr;
        (nr, nr * nr / 3.0 - 1.0)
    } else {
        (1.0, (2.0 - jr) * 2.0 / 3.0)
    };
    let mut tmp = JPLL[face] as f64 * nr + x - y;
    if tmp < 0.0 {
        tmp += 8.0;
    }
    if tmp >= 8.0 {
        tmp -= 8.0;
    }
    let phi = if nr < 1e-15 {
        0.0
    } else {
        FRAC_PI_4 * tmp / nr
    };
    (z, phi)
}

/// Face and face fractions of `(z, phi)` (the inverse of [`xyf_to_loc`];
/// the app only goes one way, the tests both).
#[cfg(test)]
pub fn loc_to_xyf(z: f64, phi: f64) -> (usize, f64, f64) {
    let za = z.abs();
    let tt = (phi / std::f64::consts::FRAC_PI_2).rem_euclid(4.0);
    if za <= 2.0 / 3.0 {
        let (t1, t2) = (0.5 + tt, z * 0.75);
        let (jp, jm) = (t1 - t2, t1 + t2);
        let (ifp, ifm) = (jp.floor() as i32, jm.floor() as i32);
        let face = if ifp == ifm {
            ifp | 4
        } else if ifp < ifm {
            ifp
        } else {
            ifm + 8
        };
        return (face as usize, jm - ifm as f64, 1.0 - (jp - ifp as f64));
    }
    let ntt = (tt.floor() as i32).min(3);
    let tp = tt - ntt as f64;
    let tmp = (3.0 * (1.0 - za)).sqrt();
    let (jp, jm) = (tp * tmp, (1.0 - tp) * tmp);
    if z > 0.0 {
        (ntt as usize, 1.0 - jm, 1.0 - jp)
    } else {
        (ntt as usize + 8, jp, jm)
    }
}

/// Unit vector (sky equatorial space) of face fractions on `face`.
pub fn xyf_to_vec(x: f64, y: f64, face: usize) -> [f64; 3] {
    let (z, phi) = xyf_to_loc(x, y, face);
    let s = (1.0 - z * z).max(0.0).sqrt();
    [s * phi.sin(), z, s * phi.cos()]
}

/// Face and face fractions of a direction (sky equatorial space).
#[cfg(test)]
pub fn vec_to_xyf(v: [f64; 3]) -> (usize, f64, f64) {
    let r = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    loc_to_xyf(v[1] / r, v[0].atan2(v[2]))
}

/// The bits of `ix` in the even places and those of `iy` in the odd.
#[cfg(test)]
pub fn interleave(ix: u32, iy: u32) -> u64 {
    let mut n = 0u64;
    for b in 0..32 {
        n |= (((ix >> b) & 1) as u64) << (2 * b) | (((iy >> b) & 1) as u64) << (2 * b + 1);
    }
    n
}

/// Inverse of [`interleave`].
pub fn deinterleave(n: u64) -> (u32, u32) {
    let (mut ix, mut iy) = (0u32, 0u32);
    for b in 0..32 {
        ix |= (((n >> (2 * b)) & 1) as u32) << b;
        iy |= (((n >> (2 * b + 1)) & 1) as u32) << b;
    }
    (ix, iy)
}

/// A tile: order and nested index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tile {
    pub order: u8,
    pub index: u64,
}

impl Tile {
    /// Number of tiles at `order`.
    pub fn count(order: u8) -> u64 {
        12 << (2 * order as u64)
    }

    /// Face, and the tile's cell on it.
    pub fn face_cell(&self) -> (usize, u32, u32) {
        let per_face = 1u64 << (2 * self.order as u64);
        let (ix, iy) = deinterleave(self.index % per_face);
        ((self.index / per_face) as usize, ix, iy)
    }

    /// Direction of a point of the tile: `fx`, `fy` in `[0, 1]` along the
    /// face's x and y axes.
    pub fn point(&self, fx: f64, fy: f64) -> [f64; 3] {
        let (face, ix, iy) = self.face_cell();
        let n = (1u64 << self.order) as f64;
        xyf_to_vec((ix as f64 + fx) / n, (iy as f64 + fy) / n, face)
    }

    /// The four children at the next order.
    pub fn children(&self) -> [Tile; 4] {
        [0, 1, 2, 3].map(|i| Tile {
            order: self.order + 1,
            index: self.index * 4 + i,
        })
    }

    /// Bounding cone: the center direction and an angle (radians) that
    /// contains the whole tile (the corner farthest from the center, plus
    /// a margin for the curved edges).
    pub fn cone(&self) -> ([f64; 3], f64) {
        let c = self.point(0.5, 0.5);
        let mut r: f64 = 0.0;
        for (fx, fy) in [
            (0.0, 0.0),
            (1.0, 0.0),
            (0.0, 1.0),
            (1.0, 1.0),
            (0.5, 0.0),
            (0.0, 0.5),
            (1.0, 0.5),
            (0.5, 1.0),
        ] {
            r = r.max(angle(c, self.point(fx, fy)));
        }
        (c, r * 1.1)
    }
}

pub fn angle(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let c = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt().atan2(d)
}

/// Tiles of `order` whose bounding cone comes within `radius` of `center`
/// (a cone of the sky, e.g. the view), found by descending from the 12
/// base tiles, so the cost follows the answer, not the sphere.
pub fn tiles_in_cone(order: u8, center: [f64; 3], radius: f64) -> Vec<Tile> {
    let mut out = Vec::new();
    let mut stack: Vec<Tile> = (0..12).map(|index| Tile { order: 0, index }).collect();
    while let Some(t) = stack.pop() {
        let (c, r) = t.cone();
        if angle(c, center) > radius + r {
            continue;
        }
        if t.order == order {
            out.push(t);
        } else {
            stack.extend(t.children());
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{PI, TAU};

    #[test]
    fn base_face_centers_are_the_known_ones() {
        // Nside 1: faces 0-3 centered at z = 2/3, faces 4-7 on the equator,
        // 8-11 at z = -2/3, at 45 degrees plus multiples of 90 (4-7 at 0).
        for f in 0..12 {
            let (z, phi) = xyf_to_loc(0.5, 0.5, f);
            let (want_z, want_phi) = match f / 4 {
                0 => (2.0 / 3.0, PI / 4.0 + (f % 4) as f64 * PI / 2.0),
                1 => (0.0, (f % 4) as f64 * PI / 2.0),
                _ => (-2.0 / 3.0, PI / 4.0 + (f % 4) as f64 * PI / 2.0),
            };
            assert!((z - want_z).abs() < 1e-12, "face {f} z {z}");
            assert!((phi - want_phi).abs() < 1e-12, "face {f} phi {phi}");
        }
    }

    #[test]
    fn locations_round_trip() {
        let mut seed = 12345u64;
        let mut rnd = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..5000 {
            let (z, phi) = (rnd() * 2.0 - 1.0, rnd() * TAU);
            let (f, x, y) = loc_to_xyf(z, phi);
            assert!((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y));
            let (z2, p2) = xyf_to_loc(x, y, f);
            let dp = (phi - p2 + PI).rem_euclid(TAU) - PI;
            assert!(
                (z - z2).abs() < 1e-9 && dp.abs() < 1e-9,
                "{z} {phi} -> {f} {x} {y}"
            );
        }
        // The poles.
        assert!((xyf_to_loc(1.0, 1.0, 0).0 - 1.0).abs() < 1e-12);
        assert!((xyf_to_loc(0.0, 0.0, 8).0 + 1.0).abs() < 1e-12);
    }

    #[test]
    fn nested_indices() {
        for (ix, iy) in [(0, 0), (5, 3), (511, 0), (123, 456), (1 << 20, 7)] {
            assert_eq!(deinterleave(interleave(ix, iy)), (ix, iy));
        }
        assert_eq!(interleave(1, 0), 1);
        assert_eq!(interleave(0, 1), 2);
        assert_eq!(interleave(3, 3), 15);
        let t = Tile {
            order: 3,
            index: 100,
        };
        assert_eq!(t.face_cell(), (1, 2, 4), "100 = 64 + 36; 36 = 0b100100");
        assert_eq!(Tile::count(3), 768);
        // Children are inside their parent.
        let (pc, pr) = t.cone();
        for c in t.children() {
            assert!(angle(c.point(0.5, 0.5), pc) < pr);
        }
    }

    #[test]
    fn tiles_cover_the_sky_and_a_cone() {
        // All of order 2 when the cone is the whole sky.
        assert_eq!(tiles_in_cone(2, [0.0, 1.0, 0.0], PI).len(), 192);
        // A small cone: the tile holding its center is among them, and far
        // tiles are not.
        let center = crate::sky::radec_vec(1.0, 0.3);
        let found = tiles_in_cone(6, center, 0.02);
        let (f, x, y) = vec_to_xyf(center);
        let n = 64.0;
        let own = Tile {
            order: 6,
            index: f as u64 * 4096 + interleave((x * n) as u32, (y * n) as u32),
        };
        assert!(found.contains(&own));
        assert!(found.len() < 20, "{}", found.len());
        for t in &found {
            assert!(angle(t.point(0.5, 0.5), center) < 0.1);
        }
    }
}
