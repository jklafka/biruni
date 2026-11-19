//! Streaming detail tiles into the image layers: which pack directories and
//! tiles to fetch each frame, and what is resident. Design:
//! `docs/tile-streaming.md`.
//!
//! [`TileStream`] is browser-free bookkeeping (tested natively). The app
//! carries out its [`Plan`] with [`fetch_range`] and [`decode_tile`], and
//! reports back with [`TileStream::dir_loaded`], [`TileStream::tile_arrived`]
//! and [`TileStream::tile_failed`]. Tiles are keyed by tileset index, so
//! switching layers and back keeps what is still cached; a lost context
//! bumps the epoch, which drops every answer still on its way.

use crate::tile_pack::{header_len, Directory};
use crate::tiles::{page_table, Levels, TileCache, TileId, Tileset};
use std::collections::HashSet;
use std::rc::Rc;

/// Tile requests in flight at once.
pub const MAX_IN_FLIGHT: usize = 6;

#[derive(Debug)]
enum Dir {
    Loading,
    Ready(Rc<Directory>),
    Failed,
}

/// A directory to fetch: the first `len` bytes of `url`.
#[derive(Debug, PartialEq)]
pub struct DirFetch {
    pub set: usize,
    pub url: String,
    pub len: u32,
    pub min: u8,
    pub max: u8,
}

/// A tile to fetch: `len` bytes of `url` from `offset`.
#[derive(Debug, PartialEq)]
pub struct TileFetch {
    pub set: usize,
    pub id: TileId,
    pub url: String,
    pub offset: u32,
    pub len: u32,
    pub epoch: u32,
}

#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub dir: Option<DirFetch>,
    pub tiles: Vec<TileFetch>,
}

/// What the renderer should stream, when it changed: the levels and page
/// table of the shown layer, or `None` for no tiles.
pub type PageUpdate = Option<(Levels, Vec<Vec<[u8; 4]>>)>;

#[derive(Debug)]
pub struct TileStream {
    sets: Vec<Tileset>,
    dirs: Vec<Option<Dir>>,
    cache: TileCache,
    /// Shown tileset and its levels on this device.
    active: Option<(usize, Levels)>,
    frame: u64,
    epoch: u32,
    in_flight: HashSet<(usize, TileId)>,
    /// Tiles the pack lacks, or that failed to load; not asked for again.
    absent: HashSet<(usize, TileId)>,
    /// Last selection, for [`Self::settled`].
    wanted: Vec<TileId>,
    pages_dirty: bool,
}

impl TileStream {
    pub fn new(capacity: usize) -> TileStream {
        TileStream {
            sets: Vec::new(),
            dirs: Vec::new(),
            cache: TileCache::new(capacity),
            active: None,
            frame: 0,
            epoch: 0,
            in_flight: HashSet::new(),
            absent: HashSet::new(),
            wanted: Vec::new(),
            pages_dirty: false,
        }
    }

    pub fn set_tilesets(&mut self, sets: Vec<Tileset>) {
        self.dirs = sets.iter().map(|_| None).collect();
        self.sets = sets;
        self.active = None;
        self.pages_dirty = true;
    }

    /// Plan this frame's work. `shown` is the layer id of the image layer on
    /// screen whose base picture is resident, at pyramid level `base`;
    /// `select` picks tiles for the view (`tiles::select`) given the levels
    /// and the cache capacity.
    pub fn plan(
        &mut self,
        shown: Option<(&str, u8)>,
        select: impl FnOnce(Levels, usize) -> Vec<TileId>,
    ) -> Plan {
        self.frame += 1;
        let mut plan = Plan::default();
        let active = shown.and_then(|(layer, base)| {
            let i = self.sets.iter().position(|s| s.layer == layer)?;
            Some((i, Levels::new(base, self.sets[i].min, self.sets[i].max)?))
        });
        if active != self.active {
            self.active = active;
            self.pages_dirty = true;
        }
        self.wanted.clear();
        let Some((i, levels)) = active else {
            return plan;
        };
        let dir = match &self.dirs[i] {
            None => {
                let s = &self.sets[i];
                self.dirs[i] = Some(Dir::Loading);
                plan.dir = Some(DirFetch {
                    set: i,
                    url: s.url.clone(),
                    len: header_len(s.min, s.max) as u32,
                    min: s.min,
                    max: s.max,
                });
                return plan;
            }
            Some(Dir::Ready(d)) => d.clone(),
            Some(Dir::Loading | Dir::Failed) => return plan,
        };
        self.wanted = select(levels, self.cache.capacity());
        self.cache.touch(i as u32, &self.wanted, self.frame);
        for &t in &self.wanted {
            if self.in_flight.len() >= MAX_IN_FLIGHT {
                break;
            }
            let key = (i, t);
            if self.cache.slot(i as u32, t).is_some()
                || self.in_flight.contains(&key)
                || self.absent.contains(&key)
            {
                continue;
            }
            match dir.range(t) {
                Some((offset, len)) => {
                    self.in_flight.insert(key);
                    plan.tiles.push(TileFetch {
                        set: i,
                        id: t,
                        url: self.sets[i].url.clone(),
                        offset,
                        len,
                        epoch: self.epoch,
                    });
                }
                None => {
                    self.absent.insert(key);
                }
            }
        }
        plan
    }

    pub fn dir_loaded(&mut self, set: usize, dir: Result<Directory, String>) {
        let Some(slot) = self.dirs.get_mut(set) else {
            return;
        };
        *slot = Some(match dir {
            Ok(d) => Dir::Ready(Rc::new(d)),
            Err(_) => Dir::Failed,
        });
    }

    /// A tile's bytes arrived and decoded: the slot to upload it to, or
    /// `None` to drop it (a stale epoch, or no slot free).
    pub fn tile_arrived(&mut self, f: &TileFetch) -> Option<u16> {
        if f.epoch != self.epoch {
            return None;
        }
        self.in_flight.remove(&(f.set, f.id));
        let slot = self.cache.insert(f.set as u32, f.id, self.frame)?;
        if self.active.is_some_and(|(i, _)| i == f.set) {
            self.pages_dirty = true;
        }
        Some(slot)
    }

    /// A tile could not be fetched, decoded or uploaded: show its parent.
    pub fn tile_failed(&mut self, f: &TileFetch) {
        if f.epoch != self.epoch {
            return;
        }
        self.in_flight.remove(&(f.set, f.id));
        self.cache.remove(f.set as u32, f.id);
        self.absent.insert((f.set, f.id));
        self.pages_dirty = true;
    }

    /// The GPU tiles are gone: forget them and anything still in flight.
    pub fn context_lost(&mut self) {
        self.cache.clear();
        self.in_flight.clear();
        self.epoch = self.epoch.wrapping_add(1);
        self.active = None;
        self.pages_dirty = true;
    }

    /// The page table to upload, if it changed since the last call.
    pub fn take_pages(&mut self) -> Option<PageUpdate> {
        if !std::mem::take(&mut self.pages_dirty) {
            return None;
        }
        Some(
            self.active
                .map(|(i, levels)| (levels, page_table(levels, |t| self.cache.slot(i as u32, t)))),
        )
    }

    /// Whether the shown layer has every tile the view wants that it can
    /// get, with nothing in flight ("off" when it streams no tiles).
    pub fn status(&self) -> &'static str {
        let Some((i, _)) = self.active else {
            return "off";
        };
        let done = self.in_flight.is_empty()
            && matches!(self.dirs[i], Some(Dir::Ready(_)) | Some(Dir::Failed))
            && self
                .wanted
                .iter()
                .all(|&t| self.cache.slot(i as u32, t).is_some() || self.absent.contains(&(i, t)));
        if done {
            "settled"
        } else {
            "loading"
        }
    }

    /// Number of resident tiles (all tilesets).
    #[cfg(test)]
    pub fn resident(&self) -> usize {
        self.cache.len()
    }
}

/// Fetch `len` bytes of `url` from `offset` with a range request. A server
/// that ignores the range and sends the whole file still works.
pub async fn fetch_range(
    url: &str,
    offset: u32,
    len: u32,
) -> Result<Vec<u8>, wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    if len == 0 {
        return Err("empty range".into());
    }
    let window = web_sys::window().ok_or("no window")?;
    let headers = js_sys::Object::new();
    let end = u64::from(offset) + u64::from(len) - 1;
    js_sys::Reflect::set(
        &headers,
        &"Range".into(),
        &format!("bytes={offset}-{end}").into(),
    )?;
    let init = web_sys::RequestInit::new();
    init.set_headers(&headers);
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_str_and_init(url, &init))
        .await?
        .dyn_into()?;
    let status = resp.status();
    if status != 206 && status != 200 {
        return Err(format!("{url}: HTTP {status}").into());
    }
    let buf = JsFuture::from(resp.array_buffer()?).await?;
    let body = js_sys::Uint8Array::new(&buf).to_vec();
    let bytes = if status == 206 {
        body
    } else {
        // Whole file: cut the range out.
        let start = offset as usize;
        body.get(start..start + len as usize)
            .ok_or_else(|| {
                wasm_bindgen::JsValue::from_str(&format!("{url}: shorter than expected"))
            })?
            .to_vec()
    };
    if bytes.len() != len as usize {
        return Err(format!("{url}: got {} bytes, expected {len}", bytes.len()).into());
    }
    Ok(bytes)
}

/// Load `tilesets.json` (see `scripts/make_tiles.py`).
pub async fn load_tilesets() -> Result<Vec<Tileset>, wasm_bindgen::JsValue> {
    use wasm_bindgen::{JsCast, JsValue};
    let bytes = crate::data::fetch("tilesets.json").await?;
    let text =
        String::from_utf8(bytes).map_err(|_| JsValue::from_str("tilesets.json is not UTF-8"))?;
    let json =
        js_sys::JSON::parse(&text).map_err(|_| JsValue::from_str("tilesets.json is not JSON"))?;
    let get = |o: &JsValue, k: &str| js_sys::Reflect::get(o, &k.into());
    if get(&json, "version")?.as_f64() != Some(1.0) {
        return Err("tilesets.json: unsupported version".into());
    }
    let list: js_sys::Array = get(&json, "tilesets")?
        .dyn_into()
        .map_err(|_| JsValue::from_str("tilesets.json: no list"))?;
    let mut sets = Vec::new();
    for t in list.iter() {
        let s = |k: &str| {
            get(&t, k)
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default()
        };
        let n = |k: &str| get(&t, k).ok().and_then(|v| v.as_f64()).unwrap_or(f64::NAN);
        let set = Tileset::new(
            &s("layer"),
            &s("url"),
            n("min_level"),
            n("max_level"),
            n("tile_size"),
        )?;
        sets.push(set);
    }
    Ok(sets)
}

/// Decode one WebP tile off the main thread.
pub async fn decode_tile(bytes: &[u8]) -> Result<web_sys::ImageBitmap, wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast;
    let window = web_sys::window().ok_or("no window")?;
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let props = web_sys::BlobPropertyBag::new();
    props.set_type("image/webp");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &props)?;
    wasm_bindgen_futures::JsFuture::from(window.create_image_bitmap_with_blob(&blob)?)
        .await?
        .dyn_into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TINY: &[u8] = include_bytes!("../scripts/testdata/tiny.tiles");

    fn id(z: u8, x: u32, y: u32) -> TileId {
        TileId { z, x, y }
    }

    /// A stream over the tiny fixture pack (levels 0..1), with a base at
    /// level 0 for the "imagery" layer.
    fn stream(capacity: usize) -> TileStream {
        let mut s = TileStream::new(capacity);
        s.set_tilesets(vec![
            Tileset::new("night", "https://x/night.tiles", 0.0, 1.0, 256.0).unwrap(),
            Tileset::new("imagery", "https://x/imagery.tiles", 0.0, 1.0, 256.0).unwrap(),
        ]);
        s
    }

    fn ready(s: &mut TileStream, set: usize) {
        s.dir_loaded(set, Directory::parse(TINY, 0, 1));
    }

    #[test]
    fn fetches_the_directory_once_then_tiles_in_order() {
        let mut s = stream(8);
        let shown = Some(("imagery", 0));
        let p = s.plan(shown, |_, _| panic!("no selection before the directory"));
        assert_eq!(
            p.dir,
            Some(DirFetch {
                set: 1,
                url: "https://x/imagery.tiles".into(),
                len: 96,
                min: 0,
                max: 1
            })
        );
        assert_eq!(
            s.plan(shown, |_, _| vec![]),
            Plan::default(),
            "still loading"
        );
        ready(&mut s, 1);
        let want = vec![id(1, 0, 0), id(1, 1, 0), id(1, 3, 1)];
        let p = s.plan(shown, |levels, cap| {
            assert_eq!((levels.base, levels.first, levels.max), (0, 1, 1));
            assert_eq!(cap, 8);
            want.clone()
        });
        assert!(p.dir.is_none());
        // (1, 3, 1) is absent from the pack: not fetched.
        let ids: Vec<TileId> = p.tiles.iter().map(|f| f.id).collect();
        assert_eq!(ids, vec![id(1, 0, 0), id(1, 1, 0)]);
        assert_eq!(
            &TINY[p.tiles[0].offset as usize..][..p.tiles[0].len as usize],
            b"z1x0y0"
        );
        assert_eq!(s.status(), "loading");
        // In flight: not asked for again.
        assert!(s.plan(shown, |_, _| want.clone()).tiles.is_empty());
        assert_eq!(s.tile_arrived(&p.tiles[0]), Some(0));
        assert_eq!(s.tile_arrived(&p.tiles[1]), Some(1));
        assert!(s.plan(shown, |_, _| want.clone()).tiles.is_empty());
        assert_eq!(s.status(), "settled");
        assert_eq!(s.resident(), 2);
    }

    #[test]
    fn page_table_follows_residency_and_the_shown_layer() {
        let mut s = stream(8);
        assert_eq!(s.take_pages(), Some(None));
        assert_eq!(s.take_pages(), None, "only when changed");
        let shown = Some(("imagery", 0));
        s.plan(shown, |_, _| vec![]);
        ready(&mut s, 1);
        let p = s.plan(shown, |_, _| vec![id(1, 2, 0)]);
        let (levels, table) = s.take_pages().unwrap().unwrap();
        assert_eq!(levels.first, 1);
        assert!(table[0].iter().all(|c| c[2] == 0), "nothing resident yet");
        s.tile_arrived(&p.tiles[0]);
        let (_, table) = s.take_pages().unwrap().unwrap();
        assert_eq!(table[0][2], [0, 0, 1, 255]);
        // A layer without tiles, or no resident base picture: tiles off.
        s.plan(Some(("terrain", 0)), |_, _| vec![]);
        assert_eq!(s.take_pages(), Some(None));
        s.plan(None, |_, _| vec![]);
        assert_eq!(s.take_pages(), None);
        assert_eq!(s.status(), "off");
        // Back on imagery: the tile is still cached.
        s.plan(shown, |_, _| vec![id(1, 2, 0)]);
        let (_, table) = s.take_pages().unwrap().unwrap();
        assert_eq!(table[0][2], [0, 0, 1, 255]);
    }

    #[test]
    fn a_base_at_the_finest_level_streams_nothing() {
        let mut s = stream(8);
        let p = s.plan(Some(("imagery", 1)), |_, _| panic!());
        assert_eq!(p, Plan::default());
        assert_eq!(s.status(), "off");
    }

    #[test]
    fn limits_requests_in_flight() {
        let mut s = TileStream::new(64);
        s.set_tilesets(vec![Tileset::new(
            "imagery",
            "https://x/a.tiles",
            0.0,
            1.0,
            256.0,
        )
        .unwrap()]);
        s.plan(Some(("imagery", 0)), |_, _| vec![]);
        ready(&mut s, 0);
        let all: Vec<TileId> = (0..4)
            .flat_map(|x| (0..2).map(move |y| id(1, x, y)))
            .collect();
        let p = s.plan(Some(("imagery", 0)), |_, _| all.clone());
        assert_eq!(p.tiles.len(), MAX_IN_FLIGHT);
        s.tile_arrived(&p.tiles[0]);
        let p2 = s.plan(Some(("imagery", 0)), |_, _| all.clone());
        // One slot opened; (1, 3, 1) is absent from the pack.
        assert_eq!(p2.tiles.len(), 1);
    }

    #[test]
    fn failures_fall_back_and_are_not_retried() {
        let mut s = stream(8);
        let shown = Some(("imagery", 0));
        s.plan(shown, |_, _| vec![]);
        ready(&mut s, 1);
        let p = s.plan(shown, |_, _| vec![id(1, 0, 0)]);
        s.tile_failed(&p.tiles[0]);
        assert!(s.plan(shown, |_, _| vec![id(1, 0, 0)]).tiles.is_empty());
        assert_eq!(s.status(), "settled");
        // A failed directory: no tiles, and settled.
        let mut s = stream(8);
        s.plan(shown, |_, _| vec![]);
        s.dir_loaded(1, Err("bad".into()));
        assert!(s.plan(shown, |_, _| panic!()).tiles.is_empty());
        assert_eq!(s.status(), "settled");
    }

    #[test]
    fn context_loss_drops_late_answers_and_refetches() {
        let mut s = stream(8);
        let shown = Some(("imagery", 0));
        s.plan(shown, |_, _| vec![]);
        ready(&mut s, 1);
        let p = s.plan(shown, |_, _| vec![id(1, 0, 0), id(1, 1, 0)]);
        assert_eq!(s.tile_arrived(&p.tiles[0]), Some(0));
        s.context_lost();
        assert_eq!(s.tile_arrived(&p.tiles[1]), None, "from before the loss");
        s.tile_failed(&p.tiles[1]); // also ignored
        assert_eq!(s.resident(), 0);
        let p = s.plan(shown, |_, _| vec![id(1, 0, 0), id(1, 1, 0)]);
        assert_eq!(
            p.tiles.len(),
            2,
            "both fetched again; the directory is kept"
        );
        assert_eq!(s.take_pages().unwrap().unwrap().0.first, 1);
    }
}
