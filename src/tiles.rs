//! Tile streaming for the image layers, browser-free: the pyramid, choosing
//! tiles for a view, the GPU slot cache and the page table.
//!
//! Design: `docs/tile-streaming.md`. Level `z` covers the world with
//! `2^(z+1) x 2^z` tiles of [`TILE_SIZE`] texels, equirectangular like every
//! other texture (column 0 starts at longitude -180, row 0 at the north
//! pole). The layer's whole-world picture is the base, level `B`; tiles add
//! the levels above it. The shader (`tileSample` in `globe.frag`) computes
//! the same level as [`detail_level`] and reads the page table built by
//! [`page_table`].

use crate::camera::Camera;
use std::collections::HashMap;
use std::f64::consts::PI;

/// Texels of the world one tile covers.
pub const TILE_SIZE: u32 = 256;
/// Texels each stored tile repeats from its neighbors on every side, so
/// bilinear filtering blends across tile edges (`tile_pack.py` BORDER).
pub const TILE_BORDER: u32 = 1;
/// Width and height of a stored tile image and of a tile array layer.
pub const TILE_STORED: u32 = TILE_SIZE + 2 * TILE_BORDER;

/// One tile of the pyramid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

/// (columns, rows) of level `z`.
pub fn grid(z: u8) -> (u32, u32) {
    (2 << z, 1 << z)
}

/// Texels around the equator at level `z`.
pub fn world_width(z: u8) -> f64 {
    (TILE_SIZE as f64) * (2u64 << z) as f64
}

/// The pyramid level of a whole-world picture `width` texels wide (4096 is
/// level 3), rounded down for widths that are not a power of two.
pub fn base_level(width: u32) -> u8 {
    let tiles = (width / (2 * TILE_SIZE)).max(1);
    (31 - tiles.leading_zeros()) as u8
}

impl TileId {
    /// The tile at `z` that contains a point (radians). Longitude wraps; the
    /// poles belong to the first and last rows.
    pub fn at(z: u8, lat: f64, lon: f64) -> TileId {
        let (cols, rows) = grid(z);
        let u = (lon / (2.0 * PI) + 0.5).rem_euclid(1.0);
        let v = (0.5 - lat / PI).clamp(0.0, 1.0);
        TileId {
            z,
            x: ((u * cols as f64) as u32).min(cols - 1),
            y: ((v * rows as f64) as u32).min(rows - 1),
        }
    }

    pub fn parent(self) -> TileId {
        debug_assert!(self.z > 0);
        TileId {
            z: self.z - 1,
            x: self.x / 2,
            y: self.y / 2,
        }
    }
}

/// The levels a layer streams on this device: tiles from `first` to `max`
/// over a base picture at level `base`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Levels {
    pub base: u8,
    pub first: u8,
    pub max: u8,
}

impl Levels {
    /// `None` when the pack has nothing finer than the base.
    pub fn new(base: u8, pack_min: u8, pack_max: u8) -> Option<Levels> {
        let first = pack_min.max(base + 1);
        (first <= pack_max).then_some(Levels {
            base,
            first,
            max: pack_max,
        })
    }
}

/// The continuous pyramid level at which one texel covers one device pixel,
/// from the texture-space derivatives along the screen axes (`u` already
/// scaled by `max(cos lat, 0.02)`, as in the shader). Mirrors `globe.frag`.
pub fn detail_level(duv_dx: (f64, f64), duv_dy: (f64, f64), max: u8) -> f64 {
    let w = world_width(max);
    let rho = (duv_dx.0 * w)
        .hypot(duv_dx.1 * w / 2.0)
        .max((duv_dy.0 * w).hypot(duv_dy.1 * w / 2.0));
    max as f64 - rho.max(1e-12).log2()
}

/// Levels subtracted from [`detail_level`] before choosing tiles; the shader
/// uses the same (`u_tileLevels.w`). Positive trades sharpness for less
/// shimmer and fewer tiles.
pub const LOD_BIAS: f64 = 0.0;

/// Screen sampling step for [`select`], CSS pixels. A tile at the level it is
/// wanted at is at least 128 device pixels across, so this cannot miss one.
const STEP: f64 = 32.0;
/// Distance to the neighbor picks that give the derivatives, CSS pixels.
const PROBE: f64 = 2.0;
/// Fraction of the cache a selection may fill, so tiles that arrive for the
/// next view still find a slot that is not wanted.
const FILL: f64 = 0.8;

/// One screen sample: where it lands, how far from the center (CSS pixels)
/// and its unbiased detail level.
struct Sample {
    lat: f64,
    lon: f64,
    dist: f64,
    level: f64,
}

fn samples(cam: &Camera, dpr: f64, max: u8) -> Vec<Sample> {
    let (w, h) = (cam.width, cam.height);
    if !(w > 0.0 && h > 0.0 && dpr > 0.0) {
        return Vec::new();
    }
    let axis = |len: f64| {
        let n = (len / STEP).ceil() as usize;
        (0..=n).map(move |i| (i as f64 * STEP).min(len - PROBE))
    };
    let mut out = Vec::new();
    for sy in axis(h) {
        for sx in axis(w) {
            let (Some(p), Some(px), Some(py)) = (
                cam.pick(sx, sy),
                cam.pick(sx + PROBE, sy),
                cam.pick(sx, sy + PROBE),
            ) else {
                continue;
            };
            let per_px = 1.0 / (PROBE * dpr);
            let scale = p.0.cos().max(0.02);
            let du =
                |q: (f64, f64)| crate::camera::wrap_angle(q.1 - p.1) / (2.0 * PI) * scale * per_px;
            let dv = |q: (f64, f64)| (p.0 - q.0) / PI * per_px;
            let level = detail_level((du(px), dv(px)), (du(py), dv(py)), max);
            if level.is_finite() && p.0.is_finite() && p.1.is_finite() {
                let dist = (sx - w / 2.0).hypot(sy - h / 2.0);
                out.push(Sample {
                    lat: p.0,
                    lon: p.1,
                    dist,
                    level,
                });
            }
        }
    }
    out
}

fn gather(samples: &[Sample], levels: Levels) -> Vec<TileId> {
    // Tile -> distance of its nearest sample.
    let mut found: HashMap<TileId, f64> = HashMap::new();
    let mut add = |t: TileId, d: f64| {
        let e = found.entry(t).or_insert(d);
        *e = e.min(d);
    };
    for s in samples {
        let d = s.level - LOD_BIAS;
        if d <= levels.base as f64 {
            continue;
        }
        let zf = (d.ceil() as i64).clamp(levels.first as i64, levels.max as i64) as u8;
        let t = TileId::at(zf, s.lat, s.lon);
        add(t, s.dist);
        if zf > levels.first {
            add(t.parent(), s.dist);
        }
    }
    let mut tiles: Vec<(TileId, f64)> = found.into_iter().collect();
    tiles.sort_by(|a, b| {
        (a.0.z, a.1, a.0)
            .partial_cmp(&(b.0.z, b.1, b.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    tiles.into_iter().map(|(t, _)| t).collect()
}

/// Choose the tiles for a view, most important first: coarse levels, then
/// those nearest the screen center. `capacity` is the number of GPU slots.
/// A crowded view (a whole globe on a large screen, the poles, where tiles
/// are narrow on the ground) keeps the coarse and central tiles; the shader
/// shows the rest from their resident parents or the base.
pub fn select(cam: &Camera, dpr: f64, levels: Levels, capacity: usize) -> Vec<TileId> {
    let mut tiles = gather(&samples(cam, dpr, levels.max), levels);
    tiles.truncate((capacity as f64 * FILL) as usize);
    tiles
}

/// Page table texels for levels `first..=max` (index 0 is `first`): each
/// cell holds the slot (R + 256 G) and level (B) of the finest resident tile
/// at its level or coarser that covers it, or level 0 for none. The texture
/// stores them as mip `max - z`.
pub fn page_table(levels: Levels, slot_of: impl Fn(TileId) -> Option<u16>) -> Vec<Vec<[u8; 4]>> {
    let mut out: Vec<Vec<[u8; 4]>> = Vec::new();
    for z in levels.first..=levels.max {
        let (cols, rows) = grid(z);
        let mut cells = Vec::with_capacity((cols * rows) as usize);
        for y in 0..rows {
            for x in 0..cols {
                let t = TileId { z, x, y };
                let cell = match slot_of(t) {
                    Some(slot) => [(slot & 0xff) as u8, (slot >> 8) as u8, z, 255],
                    None if z > levels.first => {
                        let parent = &out[(z - 1 - levels.first) as usize];
                        parent[((y / 2) * (cols / 2) + x / 2) as usize]
                    }
                    None => [0; 4],
                };
                cells.push(cell);
            }
        }
        out.push(cells);
    }
    out
}

/// GPU tile slots: which tile of which tileset each holds, evicting the least
/// recently wanted. Tiles wanted in the current frame are never evicted.
#[derive(Debug)]
pub struct TileCache {
    slots: Vec<Option<(u32, TileId)>>,
    last_wanted: Vec<u64>,
    index: HashMap<(u32, TileId), u16>,
}

impl TileCache {
    pub fn new(capacity: usize) -> TileCache {
        let capacity = capacity.min(u16::MAX as usize);
        TileCache {
            slots: vec![None; capacity],
            last_wanted: vec![0; capacity],
            index: HashMap::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    pub fn slot(&self, set: u32, t: TileId) -> Option<u16> {
        self.index.get(&(set, t)).copied()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Mark resident tiles as wanted in `frame`.
    pub fn touch(&mut self, set: u32, tiles: &[TileId], frame: u64) {
        for &t in tiles {
            if let Some(s) = self.slot(set, t) {
                self.last_wanted[s as usize] = frame;
            }
        }
    }

    /// A slot for a newly arrived tile: a free one, else the least recently
    /// wanted one that is not wanted in `frame`. `None` if every slot is.
    pub fn insert(&mut self, set: u32, t: TileId, frame: u64) -> Option<u16> {
        if let Some(s) = self.slot(set, t) {
            return Some(s);
        }
        let free = self.slots.iter().position(Option::is_none);
        let slot = free.or_else(|| {
            (0..self.slots.len())
                .filter(|&i| self.last_wanted[i] < frame)
                .min_by_key(|&i| self.last_wanted[i])
        })?;
        if let Some(old) = self.slots[slot].take() {
            self.index.remove(&old);
        }
        self.slots[slot] = Some((set, t));
        self.last_wanted[slot] = frame;
        self.index.insert((set, t), slot as u16);
        Some(slot as u16)
    }

    /// Free the slot of a tile, if it has one.
    pub fn remove(&mut self, set: u32, t: TileId) {
        if let Some(s) = self.index.remove(&(set, t)) {
            self.slots[s as usize] = None;
        }
    }

    /// Forget everything (the GPU texture is gone).
    pub fn clear(&mut self) {
        self.slots.iter_mut().for_each(|s| *s = None);
        self.index.clear();
    }
}

/// A tile pack from `tilesets.json`, checked.
#[derive(Clone, Debug, PartialEq)]
pub struct Tileset {
    pub layer: String,
    pub url: String,
    pub min: u8,
    pub max: u8,
}

impl Tileset {
    pub fn new(
        layer: &str,
        url: &str,
        min: f64,
        max: f64,
        tile_size: f64,
    ) -> Result<Tileset, String> {
        let level = |v: f64| {
            (v.fract() == 0.0 && (0.0..=crate::tile_pack::MAX_LEVEL as f64).contains(&v))
                .then_some(v as u8)
        };
        let (Some(min), Some(max)) = (level(min), level(max)) else {
            return Err(format!("tilesets.json: bad levels for {layer}"));
        };
        if min > max {
            return Err(format!("tilesets.json: bad levels for {layer}"));
        }
        if tile_size != TILE_SIZE as f64 {
            return Err(format!(
                "tilesets.json: {layer} tiles are not {TILE_SIZE} px"
            ));
        }
        if !url.starts_with("https://") {
            return Err(format!("tilesets.json: {layer} url is not https"));
        }
        Ok(Tileset {
            layer: layer.to_string(),
            url: url.to_string(),
            min,
            max,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use crate::projection::Projection;

    fn id(z: u8, x: u32, y: u32) -> TileId {
        TileId { z, x, y }
    }

    fn cam(lat: f64, lon: f64, km: f64) -> Camera {
        let mut c = Camera::new(lat, lon, km / crate::camera::EARTH_RADIUS_KM);
        c.width = 1280.0;
        c.height = 800.0;
        c
    }

    const DESKTOP: Levels = Levels {
        base: 3,
        first: 4,
        max: 5,
    };

    #[test]
    fn pyramid_math() {
        assert_eq!(grid(0), (2, 1));
        assert_eq!(grid(5), (64, 32));
        assert_eq!(world_width(3), 4096.0);
        assert_eq!(base_level(4096), 3);
        assert_eq!(base_level(2048), 2);
        assert_eq!(base_level(5000), 3);
        assert_eq!(base_level(0), 0);
        assert_eq!(
            TileId::at(5, 46.8f64.to_radians(), 8.2f64.to_radians()),
            id(5, 33, 7)
        );
        assert_eq!(id(5, 33, 7).parent(), id(4, 16, 3));
    }

    #[test]
    fn tile_lookup_wraps_longitude_and_clamps_the_poles() {
        let pi = PI;
        assert_eq!(TileId::at(4, 0.0, -pi), id(4, 0, 8));
        assert_eq!(TileId::at(4, 0.0, pi), id(4, 0, 8), "+180 is -180");
        assert_eq!(TileId::at(4, 0.0, pi - 1e-9), id(4, 31, 8));
        assert_eq!(TileId::at(4, 0.0, 3.0 * pi - 1e-9), id(4, 31, 8));
        assert_eq!(TileId::at(4, pi / 2.0, 0.0), id(4, 16, 0));
        assert_eq!(TileId::at(4, -pi / 2.0, 0.0), id(4, 16, 15));
        assert_eq!(
            TileId::at(4, 2.0, 0.0),
            id(4, 16, 0),
            "beyond the pole clamps"
        );
    }

    #[test]
    fn levels_skip_what_the_base_covers() {
        assert_eq!(Levels::new(3, 3, 5), Some(DESKTOP));
        assert_eq!(
            Levels::new(2, 3, 5),
            Some(Levels {
                base: 2,
                first: 3,
                max: 5
            })
        );
        assert_eq!(Levels::new(5, 3, 5), None);
    }

    #[test]
    fn detail_level_formula() {
        // One level-5 texel per pixel along u: exactly level 5.
        let w = world_width(5);
        assert!((detail_level((1.0 / w, 0.0), (0.0, 1.0 / w), 5) - 5.0).abs() < 1e-9);
        // Twice the footprint: one level coarser. The v axis counts half the width.
        assert!((detail_level((2.0 / w, 0.0), (0.0, 0.0), 5) - 4.0).abs() < 1e-9);
        assert!((detail_level((0.0, 0.0), (0.0, 4.0 / w), 5) - 4.0).abs() < 1e-9);
        // The larger axis wins.
        assert!((detail_level((8.0 / w, 0.0), (0.0, 2.0 / w), 5) - 2.0).abs() < 1e-9);
        // Zero derivatives stay finite.
        assert!(detail_level((0.0, 0.0), (0.0, 0.0), 5).is_finite());
    }

    #[test]
    fn whole_globe_view_wants_no_tiles_on_a_small_screen() {
        let mut c = cam(20.0, 0.0, 20_000.0);
        c.width = 400.0;
        c.height = 300.0;
        assert!(select(&c, 1.0, DESKTOP, 96).is_empty());
    }

    #[test]
    fn close_zoom_wants_finest_tiles_around_the_center_with_parents() {
        let c = cam(46.8, 8.2, 400.0);
        let s = select(&c, 2.0, DESKTOP, 96);
        let center = TileId::at(5, 46.8f64.to_radians(), 8.2f64.to_radians());
        assert!(s.contains(&center));
        assert!(s.contains(&center.parent()));
        // Coarse first, then by distance: the first level-5 tile is the center's.
        let first5 = s.iter().find(|t| t.z == 5).unwrap();
        assert_eq!(*first5, center);
        assert!(s.windows(2).all(|w| w[0].z <= w[1].z));
        // Every level-5 tile's parent is wanted too.
        for t in s.iter().filter(|t| t.z == 5) {
            assert!(s.contains(&t.parent()), "{t:?}");
        }
        assert!(s.iter().all(|t| t.z >= 4 && t.z <= 5));
    }

    #[test]
    fn views_across_the_antimeridian_want_tiles_from_both_edges() {
        let c = cam(0.0, 180.0, 400.0);
        let s = select(&c, 2.0, DESKTOP, 96);
        assert!(s.iter().any(|t| t.z == 5 && t.x == 0));
        assert!(s.iter().any(|t| t.z == 5 && t.x == 63));
    }

    #[test]
    fn crowded_views_keep_coarse_and_central_tiles_within_the_cache() {
        let mut c = cam(20.0, 8.0, 12_000.0);
        c.width = 1920.0;
        c.height = 1080.0;
        let all = select(&c, 2.0, DESKTOP, 10_000);
        let s = select(&c, 2.0, DESKTOP, 32);
        assert!(all.len() > 25, "{}", all.len());
        assert_eq!(s.len(), 25);
        assert_eq!(s[..], all[..25], "a prefix of the full order");
        // The pole: narrow tiles, still within budget and including the top row.
        let c = cam(89.0, 0.0, 400.0);
        let s = select(&c, 2.0, DESKTOP, 32);
        assert!(!s.is_empty() && s.len() <= 25);
        assert!(s.iter().any(|t| t.y == 0));
    }

    #[test]
    fn flat_maps_select_too() {
        let mut c = cam(46.8, 8.2, 400.0);
        c.set_projection(Projection::Equirectangular);
        let s = select(&c, 2.0, DESKTOP, 96);
        assert!(!s.is_empty());
        assert!(s.contains(&TileId::at(5, 46.8f64.to_radians(), 8.2f64.to_radians())));
    }

    #[test]
    fn bad_input_selects_nothing() {
        let mut c = cam(46.8, 8.2, 400.0);
        assert!(select(&c, f64::NAN, DESKTOP, 96).is_empty());
        assert!(select(&c, 0.0, DESKTOP, 96).is_empty());
        c.width = 0.0;
        assert!(select(&c, 2.0, DESKTOP, 96).is_empty());
        let mut c = cam(46.8, 8.2, 400.0);
        c.lat = f64::NAN;
        assert!(select(&c, 2.0, DESKTOP, 96).is_empty());
        let c = cam(46.8, 8.2, 400.0);
        assert!(select(&c, 2.0, DESKTOP, 0).is_empty());
    }

    #[test]
    fn page_table_inherits_from_coarser_resident_tiles() {
        let levels = DESKTOP;
        let resident = [(id(4, 16, 3), 7u16), (id(5, 33, 7), 300u16)];
        let table = page_table(levels, |t| resident.iter().find(|r| r.0 == t).map(|r| r.1));
        assert_eq!(table.len(), 2);
        assert_eq!(table[0].len(), 32 * 16);
        assert_eq!(table[1].len(), 64 * 32);
        let at = |z: u8, x: u32, y: u32| table[(z - 4) as usize][(y * grid(z).0 + x) as usize];
        assert_eq!(at(4, 16, 3), [7, 0, 4, 255]);
        assert_eq!(at(4, 0, 0), [0, 0, 0, 0]);
        assert_eq!(at(5, 33, 7), [44, 1, 5, 255], "slot 300 = 44 + 256");
        // Siblings of the level-5 tile fall back to its resident parent.
        assert_eq!(at(5, 32, 6), [7, 0, 4, 255]);
        assert_eq!(at(5, 33, 6), [7, 0, 4, 255]);
        assert_eq!(at(5, 0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn cache_evicts_the_least_recently_wanted_but_never_this_frames() {
        let mut c = TileCache::new(2);
        assert_eq!(c.insert(1, id(4, 0, 0), 1), Some(0));
        assert_eq!(c.insert(1, id(4, 1, 0), 2), Some(1));
        assert_eq!(c.insert(1, id(4, 1, 0), 2), Some(1), "already resident");
        c.touch(1, &[id(4, 0, 0)], 3);
        // Slot 1 was wanted longest ago.
        assert_eq!(c.insert(1, id(4, 2, 0), 4), Some(1));
        assert_eq!(c.slot(1, id(4, 1, 0)), None);
        assert_eq!(c.slot(1, id(4, 2, 0)), Some(1));
        // Both wanted this frame: no room.
        c.touch(1, &[id(4, 0, 0), id(4, 2, 0)], 5);
        assert_eq!(c.insert(1, id(4, 3, 0), 5), None);
        // Tilesets are kept apart.
        assert_eq!(c.slot(2, id(4, 0, 0)), None);
        assert_eq!(c.insert(2, id(4, 0, 0), 6), Some(0));
        assert_eq!(c.slot(1, id(4, 0, 0)), None);
        c.clear();
        assert_eq!(c.len(), 0);
        assert_eq!(c.insert(1, id(4, 0, 0), 7), Some(0));
        assert_eq!(TileCache::new(0).insert(1, id(4, 0, 0), 1), None);
    }

    #[test]
    fn tileset_entries_are_checked() {
        let ok = Tileset::new("imagery", "https://x/imagery.1.tiles", 3.0, 5.0, 256.0).unwrap();
        assert_eq!((ok.min, ok.max), (3, 5));
        assert!(Tileset::new("imagery", "http://x/a.tiles", 3.0, 5.0, 256.0).is_err());
        assert!(Tileset::new("imagery", "https://x/a.tiles", 5.0, 3.0, 256.0).is_err());
        assert!(Tileset::new("imagery", "https://x/a.tiles", 3.5, 5.0, 256.0).is_err());
        assert!(Tileset::new("imagery", "https://x/a.tiles", 3.0, 11.0, 256.0).is_err());
        assert!(Tileset::new("imagery", "https://x/a.tiles", f64::NAN, 5.0, 256.0).is_err());
        assert!(Tileset::new("imagery", "https://x/a.tiles", 3.0, 5.0, 512.0).is_err());
    }
}
