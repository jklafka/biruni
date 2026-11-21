//! Deep-sky survey imagery for the sky view: the Digitized Sky Survey in
//! color, as HiPS tiles (IVOA Hierarchical Progressive Surveys) made by CDS
//! and served by mirrors. Browser-free planning and geometry; the app does
//! the fetching and `sky_render` the drawing.
//!
//! The survey is drawn in two layers. The Allsky atlas (every order-3 tile
//! at 64 px in one 1728x1856 JPEG) covers the whole sky in one download and
//! serves wide views. When the view is fine enough that the atlas would be
//! magnified more than about twice, tiles of the order whose texels are
//! about a screen pixel are streamed on top (fetched at 512 px, kept at 256
//! px). Missing tiles leave the atlas showing.

use crate::healpix::{angle, tiles_in_cone, Tile};
use crate::sky::{mul, transpose, Mat3, SkyCamera};
use std::collections::{HashMap, HashSet};

/// HiPS service URLs of DSS2 color, tried in order (CDS's own `alasky`
/// copy answered 404 when this was written; both of these send
/// `Access-Control-Allow-Origin: *`).
pub const MIRRORS: [&str; 2] = [
    "https://skies.esac.esa.int/DSSColor",
    "https://irsa.ipac.caltech.edu/data/hips/CDS/DSS2/color",
];
/// Deepest order of the survey (`hips_order` in its properties).
pub const MAX_ORDER: u8 = 9;
pub const ALLSKY_ORDER: u8 = 3;
/// Tiles per row of the Allsky atlas.
pub const ALLSKY_COLUMNS: u64 = 27;
/// The Allsky atlas at half size, for the small texture tier.
pub const ALLSKY_SMALL: (u32, u32) = (864, 928);
/// Tiles are uploaded at this size (the survey's are 512).
pub const TILE_PX: u32 = 256;
/// Requests in flight at once.
pub const MAX_IN_FLIGHT: usize = 6;
/// Most tiles drawn on top of the atlas in one view.
pub const MAX_VISIBLE: usize = 64;
/// Grid cells per side of a tile's mesh.
pub const TILE_MESH: usize = 8;
pub const ALLSKY_MESH: usize = 4;
/// Angular size of an order-0 tile's side, radians (sqrt(4 pi / 12)).
const SIDE0: f64 = 1.023_326_707_946_488_5;
/// The survey's sky background level, taken off before adding it to the
/// sky (0..1): the Allsky atlas averages 8/255.
pub const BLACK: f32 = 0.035;

pub fn tile_url(base: &str, t: Tile) -> String {
    let dir = t.index / 10_000 * 10_000;
    format!("{base}/Norder{}/Dir{dir}/Npix{}.jpg", t.order, t.index)
}

pub fn allsky_url(base: &str) -> String {
    format!("{base}/Norder{ALLSKY_ORDER}/Allsky.jpg")
}

/// The order to stream for a view whose pixels are `rad_per_px` radians
/// (CSS pixels at the center): `None` while the atlas is good enough (its
/// 64 px cells magnified at most twice), else the coarsest order whose 256
/// px texels are at most about 1.5 pixels, but at least the atlas's.
pub fn wanted_order(rad_per_px: f64) -> Option<u8> {
    if !(rad_per_px.is_finite() && rad_per_px > 0.0) {
        return None;
    }
    let atlas_texel = SIDE0 / (1u64 << ALLSKY_ORDER) as f64 / 64.0;
    if atlas_texel <= 2.0 * rad_per_px {
        return None;
    }
    let k = (SIDE0 / (TILE_PX as f64 * 1.5 * rad_per_px)).log2().ceil();
    Some((k.max(ALLSKY_ORDER as f64) as u8).min(MAX_ORDER))
}

/// The view as a cone of the sky: the center direction (sky equatorial
/// space) and the angle to the farthest screen corner.
pub fn view_cone(cam: &SkyCamera, eq_to_hor: &Mat3) -> Option<([f64; 3], f64)> {
    let hor_to_eq = transpose(eq_to_hor);
    let c = mul(&hor_to_eq, cam.pick(cam.width / 2.0, cam.height / 2.0)?);
    let mut r: f64 = 0.0;
    for (x, y) in [
        (0.0, 0.0),
        (cam.width, 0.0),
        (0.0, cam.height),
        (cam.width, cam.height),
    ] {
        r = r.max(angle(c, mul(&hor_to_eq, cam.pick(x, y)?)));
    }
    Some((c, r))
}

/// Texture coordinates of a point of an order-3 tile in the Allsky atlas:
/// cell `index` at column `index % 27`, row `index / 27`; inside it the
/// columns follow the face's y and the rows its x (see `healpix`). Inset
/// half a texel of a `cell_px` cell so neighbors do not bleed in.
pub fn allsky_uv(t: Tile, fx: f64, fy: f64, cell_px: f64, rows: u64) -> (f32, f32) {
    let (col, row) = (
        (t.index % ALLSKY_COLUMNS) as f64,
        (t.index / ALLSKY_COLUMNS) as f64,
    );
    let inset = 0.5 / cell_px;
    let u = (col + inset + fy * (1.0 - 2.0 * inset)) / ALLSKY_COLUMNS as f64;
    let v = (row + inset + fx * (1.0 - 2.0 * inset)) / rows as f64;
    (u as f32, v as f32)
}

/// Texture coordinates of a point in a tile's own image.
pub fn tile_uv(fx: f64, fy: f64) -> (f32, f32) {
    let inset = 0.5 / TILE_PX as f64;
    let s = 1.0 - 2.0 * inset;
    ((inset + fy * s) as f32, (inset + fx * s) as f32)
}

/// A tile's mesh: `(n + 1)^2` vertices of direction (3 floats) and texture
/// coordinates (2), and the triangles, with `first` added to every index.
pub fn mesh(
    t: Tile,
    n: usize,
    first: u16,
    uv: impl Fn(f64, f64) -> (f32, f32),
    verts: &mut Vec<f32>,
    indices: &mut Vec<u16>,
) {
    for i in 0..=n {
        for j in 0..=n {
            let (fx, fy) = (i as f64 / n as f64, j as f64 / n as f64);
            let d = t.point(fx, fy);
            let (u, v) = uv(fx, fy);
            verts.extend([d[0] as f32, d[1] as f32, d[2] as f32, u, v]);
        }
    }
    let at = |i: usize, j: usize| first + (i * (n + 1) + j) as u16;
    for i in 0..n {
        for j in 0..n {
            indices.extend([at(i, j), at(i + 1, j), at(i, j + 1)]);
            indices.extend([at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
}

/// Which order-3 cells of the atlas to draw for a view cone, as indices
/// into the atlas mesh (built by [`allsky_mesh`] in tile order).
pub fn allsky_indices(cone: ([f64; 3], f64)) -> Vec<u16> {
    let per_tile = 6 * ALLSKY_MESH * ALLSKY_MESH;
    let verts = ((ALLSKY_MESH + 1) * (ALLSKY_MESH + 1)) as u16;
    let mut out = Vec::new();
    for t in tiles_in_cone(ALLSKY_ORDER, cone.0, cone.1) {
        let mut idx = Vec::with_capacity(per_tile);
        let mut scratch = Vec::new();
        mesh(
            t,
            ALLSKY_MESH,
            t.index as u16 * verts,
            |_, _| (0.0, 0.0),
            &mut scratch,
            &mut idx,
        );
        out.extend(idx);
    }
    out
}

/// The atlas mesh: every order-3 tile, in index order.
pub fn allsky_mesh(cell_px: f64, rows: u64) -> Vec<f32> {
    let mut verts = Vec::new();
    let mut idx = Vec::new();
    let per = ((ALLSKY_MESH + 1) * (ALLSKY_MESH + 1)) as u16;
    for index in 0..Tile::count(ALLSKY_ORDER) {
        let t = Tile {
            order: ALLSKY_ORDER,
            index,
        };
        mesh(
            t,
            ALLSKY_MESH,
            index as u16 * per,
            |fx, fy| allsky_uv(t, fx, fy, cell_px, rows),
            &mut verts,
            &mut idx,
        );
    }
    verts
}

/// Streaming state for the survey tiles (the atlas is fetched once by the
/// app). Tiles are cached up to a capacity and evicted least recently used.
#[derive(Debug)]
pub struct Survey {
    capacity: usize,
    frame: u64,
    /// Cached tiles and the frame they were last wanted.
    cache: HashMap<Tile, u64>,
    in_flight: HashSet<Tile>,
    /// Tiles that failed or do not exist; not asked for again.
    absent: HashSet<Tile>,
    /// Bumped when the GPU tiles are lost, so late answers are dropped.
    pub epoch: u32,
    wanted: Vec<Tile>,
}

impl Survey {
    pub fn new(capacity: usize) -> Survey {
        Survey {
            capacity,
            frame: 0,
            cache: HashMap::new(),
            in_flight: HashSet::new(),
            absent: HashSet::new(),
            epoch: 0,
            wanted: Vec::new(),
        }
    }

    /// The tiles a view wants: those of the wanted order in the view cone,
    /// nearest the center first, at most [`MAX_VISIBLE`].
    pub fn select(order: u8, cone: ([f64; 3], f64)) -> Vec<Tile> {
        let mut t = tiles_in_cone(order, cone.0, cone.1);
        t.sort_by(|a, b| {
            let (da, db) = (
                angle(a.point(0.5, 0.5), cone.0),
                angle(b.point(0.5, 0.5), cone.0),
            );
            da.total_cmp(&db).then(a.cmp(b))
        });
        t.truncate(MAX_VISIBLE);
        t
    }

    /// Plan this frame: remember what is wanted and return the tiles to
    /// fetch now (not cached, not in flight, not known absent).
    pub fn plan(&mut self, wanted: Vec<Tile>) -> Vec<Tile> {
        self.frame += 1;
        for t in &wanted {
            if let Some(f) = self.cache.get_mut(t) {
                *f = self.frame;
            }
        }
        let mut out = Vec::new();
        for t in &wanted {
            if self.in_flight.len() >= MAX_IN_FLIGHT {
                break;
            }
            if self.cache.contains_key(t) || self.in_flight.contains(t) || self.absent.contains(t) {
                continue;
            }
            self.in_flight.insert(*t);
            out.push(*t);
        }
        self.wanted = wanted;
        out
    }

    /// A tile arrived and is uploaded: cache it, and return the tile to
    /// evict to stay within capacity (never one wanted now).
    pub fn arrived(&mut self, t: Tile) -> Option<Tile> {
        self.in_flight.remove(&t);
        self.cache.insert(t, self.frame);
        if self.cache.len() <= self.capacity {
            return None;
        }
        let victim = self
            .cache
            .iter()
            .filter(|(k, _)| !self.wanted.contains(k))
            .min_by_key(|(k, f)| (**f, **k))
            .map(|(k, _)| *k)?;
        self.cache.remove(&victim);
        Some(victim)
    }

    pub fn failed(&mut self, t: Tile) {
        self.in_flight.remove(&t);
        self.absent.insert(t);
    }

    #[cfg(test)]
    pub fn is_cached(&self, t: Tile) -> bool {
        self.cache.contains_key(&t)
    }

    /// Cached tiles to draw for a view cone, coarse first so finer ones
    /// cover them.
    pub fn drawable(&self, cone: ([f64; 3], f64)) -> Vec<Tile> {
        let mut out: Vec<Tile> = self
            .cache
            .keys()
            .filter(|t| {
                let (c, r) = t.cone();
                angle(c, cone.0) <= cone.1 + r
            })
            .copied()
            .collect();
        out.sort();
        out
    }

    /// Nothing in flight and every wanted tile cached or absent.
    pub fn settled(&self) -> bool {
        self.in_flight.is_empty()
            && self
                .wanted
                .iter()
                .all(|t| self.cache.contains_key(t) || self.absent.contains(t))
    }

    /// The GPU lost its tiles: forget them and anything in flight.
    pub fn context_lost(&mut self) {
        self.cache.clear();
        self.in_flight.clear();
        self.epoch = self.epoch.wrapping_add(1);
    }

    /// Answers from before [`Self::context_lost`] are dropped by the caller
    /// comparing epochs; this clears their in-flight mark.
    pub fn forget(&mut self, t: Tile) {
        self.in_flight.remove(&t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::{eq_to_horizon, radec_vec, vec_radec};

    #[test]
    fn urls_follow_the_hips_layout() {
        let t = Tile {
            order: 9,
            index: 1_234_567,
        };
        assert_eq!(
            tile_url("https://x/DSS", t),
            "https://x/DSS/Norder9/Dir1230000/Npix1234567.jpg"
        );
        assert_eq!(
            allsky_url("https://x/DSS"),
            "https://x/DSS/Norder3/Allsky.jpg"
        );
    }

    #[test]
    fn orders_follow_the_pixel_size() {
        // A 100-degree view on 800x600: about 0.003 rad a pixel: atlas.
        assert_eq!(wanted_order(0.003), None);
        // Just past twice magnified: the atlas's own order, from tiles.
        assert_eq!(wanted_order(9e-4), Some(3));
        // Finer views stream finer orders, up to the survey's deepest.
        let mut last = 3;
        for px in [5e-4, 2e-4, 5e-5, 1e-5, 2e-6] {
            let k = wanted_order(px).unwrap();
            assert!(k >= last);
            last = k;
            let texel = SIDE0 / (1u64 << k) as f64 / TILE_PX as f64;
            assert!(texel <= 1.5 * px || k == MAX_ORDER, "{px}: order {k}");
        }
        assert_eq!(wanted_order(1e-9), Some(MAX_ORDER));
        assert_eq!(wanted_order(f64::NAN), None);
        assert_eq!(wanted_order(0.0), None);
    }

    #[test]
    fn uv_maps_columns_along_the_face_y() {
        let t = Tile {
            order: 3,
            index: 28,
        }; // row 1, column 1
        let (u0, v0) = allsky_uv(t, 0.0, 0.0, 64.0, 29);
        let (u1, v1) = allsky_uv(t, 0.0, 1.0, 64.0, 29);
        assert!(
            u1 > u0 && (v1 - v0).abs() < 1e-7,
            "face y moves along columns"
        );
        assert!((u0 as f64 - (1.0 + 0.5 / 64.0) / 27.0).abs() < 1e-6);
        assert!((v0 as f64 - (1.0 + 0.5 / 64.0) / 29.0).abs() < 1e-6);
        let (u, v) = tile_uv(1.0, 0.0);
        assert!(u < 0.01 && v > 0.99);
    }

    #[test]
    fn meshes_cover_their_tile() {
        let t = Tile {
            order: 5,
            index: 777,
        };
        let (mut v, mut i) = (Vec::new(), Vec::new());
        mesh(t, 4, 10, tile_uv, &mut v, &mut i);
        assert_eq!(v.len(), 25 * 5);
        assert_eq!(i.len(), 4 * 4 * 6);
        assert_eq!(*i.iter().min().unwrap(), 10);
        assert_eq!(*i.iter().max().unwrap(), 34);
        let c = [v[60] as f64, v[61] as f64, v[62] as f64]; // vertex (2, 2)
        assert!(angle(c, t.point(0.5, 0.5)) < 1e-6);
        let atlas = allsky_mesh(64.0, 29);
        assert_eq!(atlas.len(), 768 * 25 * 5);
        assert!(atlas.len() / 5 < u16::MAX as usize);
    }

    #[test]
    fn the_view_cone_and_atlas_cells() {
        let m = eq_to_horizon(0.7, 1.0);
        let cam = SkyCamera::new(3.0, 0.5, 60f64.to_radians());
        let (c, r) = view_cone(&cam, &m).unwrap();
        let center = mul(&transpose(&m), crate::sky::horiz_vec(3.0, 0.5));
        assert!(angle(c, center) < 1e-9);
        // The farthest corner of 800x600 at 60 degrees across 600:
        // 2 atan(tan(15 deg) * 500 / 300) = 48.1 degrees.
        assert!((r.to_degrees() - 48.13).abs() < 0.01, "{}", r.to_degrees());
        let idx = allsky_indices((c, r));
        let cells = idx.len() / (6 * ALLSKY_MESH * ALLSKY_MESH);
        assert!(cells > 20 && cells < 300, "{cells}");
    }

    #[test]
    fn streaming_caches_evicts_and_settles() {
        let cone = (radec_vec(1.5, 0.2), 0.05);
        let wanted = Survey::select(7, cone);
        assert!(!wanted.is_empty() && wanted.len() <= MAX_VISIBLE);
        let (ra, dec) = vec_radec(wanted[0].point(0.5, 0.5));
        assert!(angle(radec_vec(ra, dec), cone.0) < 0.05, "nearest first");
        let mut s = Survey::new(3);
        let first = s.plan(wanted.clone());
        assert_eq!(first.len(), wanted.len().min(MAX_IN_FLIGHT));
        assert!(!s.settled());
        assert!(
            s.plan(wanted.clone()).len() <= MAX_IN_FLIGHT - first.len(),
            "in flight not asked again"
        );
        s.failed(first[0]);
        for t in &first[1..] {
            s.arrived(*t);
        }
        assert!(s.cache.len() <= 3 || first[1..].iter().all(|t| wanted.contains(t)));
        // Capacity: old, unwanted tiles go first.
        let mut s = Survey::new(2);
        let a = Tile { order: 6, index: 1 };
        let b = Tile { order: 6, index: 2 };
        let c = Tile { order: 6, index: 3 };
        s.plan(vec![a, b]);
        s.arrived(a);
        s.arrived(b);
        s.plan(vec![c]);
        assert_eq!(s.arrived(c), Some(a));
        assert!(s.is_cached(b) && s.is_cached(c));
        s.context_lost();
        assert!(!s.is_cached(c));
        assert_eq!(s.epoch, 1);
    }

    #[test]
    fn failed_tiles_are_not_retried_and_count_as_settled() {
        let t = Tile { order: 5, index: 9 };
        let mut s = Survey::new(8);
        assert_eq!(s.plan(vec![t]), vec![t]);
        s.failed(t);
        assert!(s.plan(vec![t]).is_empty());
        assert!(s.settled());
    }
}
