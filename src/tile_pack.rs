//! Tile pack header and directory (browser-free).
//!
//! A tile pack holds one layer's equirectangular tile pyramid; the format is
//! described in `docs/tile-streaming.md` and written by
//! `scripts/tile_pack.py`. Both parse `scripts/testdata/tiny.tiles` in their
//! tests, so the two stay in step. The directory of a deep pack is large
//! (349 KB for levels 3..7), so the app reads it in pages of [`DIR_PAGE`]
//! entries as tiles need them: the header with the first page in one range
//! request ([`first_len`]), each later page in another ([`Directory::page_range`]),
//! then each tile by its byte range.

use crate::tiles::{grid, TileId, TILE_BORDER};

const MAGIC: &[u8; 4] = b"BTIL";
const VERSION: u8 = 1;
const FORMAT_WEBP: u8 = 1;
const HEADER: usize = 16;
const ENTRY: usize = 8;
/// Deepest level a pack may have (`MAX_LEVEL` in `tile_pack.py`).
pub const MAX_LEVEL: u8 = 10;

/// Number of tiles in levels `min..=max`.
pub fn tile_count(min: u8, max: u8) -> usize {
    (min..=max).map(|z| (grid(z).0 * grid(z).1) as usize).sum()
}

/// Directory entries per page (32 KiB). The levels up to 5 fit in the first.
pub const DIR_PAGE: usize = 4096;

/// Bytes of the header plus the whole directory.
#[cfg(test)]
pub fn header_len(min: u8, max: u8) -> usize {
    HEADER + ENTRY * tile_count(min, max)
}

/// Bytes of the header plus the first directory page of `page` entries.
pub fn first_len(min: u8, max: u8, page: usize) -> usize {
    HEADER + ENTRY * tile_count(min, max).min(page)
}

#[derive(Debug)]
enum Page {
    Missing,
    Loading,
    Ready(Vec<(u32, u32)>),
    Failed,
}

/// Where a tile's bytes are, as far as the loaded pages tell.
#[derive(Debug, PartialEq)]
pub enum Lookup {
    /// Byte range (start, length) in the pack.
    Tile(u32, u32),
    /// The pack lacks the tile, it is outside the pyramid, or its directory
    /// page failed to load.
    Absent,
    /// Directory page `p` has not been asked for yet.
    NeedPage(usize),
    /// Directory page is on its way.
    Waiting,
}

/// The directory, as far as loaded: where each tile's bytes are in the pack.
/// The tile size field is not checked here (the fixture's tiles are tiny);
/// the renderer checks each decoded tile.
#[derive(Debug)]
pub struct Directory {
    pub min: u8,
    pub max: u8,
    count: usize,
    page: usize,
    pages: Vec<Page>,
}

fn entries(data: &[u8], n: usize) -> Vec<(u32, u32)> {
    let u32_at = |i: usize| u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
    (0..n)
        .map(|i| (u32_at(i * ENTRY), u32_at(i * ENTRY + 4)))
        .collect()
}

impl Directory {
    /// Parse the start of a pack (at least [`first_len`] bytes for pages of
    /// `page` entries) and check it against the levels that `tilesets.json`
    /// promised.
    pub fn parse(data: &[u8], min: u8, max: u8, page: usize) -> Result<Directory, String> {
        if page == 0 {
            return Err("tile pack: empty directory page".into());
        }
        if data.len() < HEADER {
            return Err("tile pack: truncated header".into());
        }
        if &data[0..4] != MAGIC {
            return Err("tile pack: bad magic".into());
        }
        if data[4] != VERSION {
            return Err(format!("tile pack: unsupported version {}", data[4]));
        }
        let (lo, hi, format) = (data[5], data[6], data[7]);
        if (lo, hi) != (min, max) || lo > hi || hi > MAX_LEVEL {
            return Err(format!(
                "tile pack: levels {lo}..{hi}, expected {min}..{max}"
            ));
        }
        if format != FORMAT_WEBP {
            return Err(format!("tile pack: unknown tile format {format}"));
        }
        let border = u16::from_le_bytes([data[10], data[11]]) as u32;
        if border != TILE_BORDER {
            return Err(format!(
                "tile pack: border {border}, expected {TILE_BORDER}"
            ));
        }
        let count = u32::from_le_bytes([data[12], data[13], data[14], data[15]]) as usize;
        if count != tile_count(lo, hi) {
            return Err(format!(
                "tile pack: {count} tiles, expected {}",
                tile_count(lo, hi)
            ));
        }
        if data.len() < first_len(lo, hi, page) {
            return Err("tile pack: truncated directory".into());
        }
        let n_pages = count.div_ceil(page);
        let mut pages: Vec<Page> = (0..n_pages).map(|_| Page::Missing).collect();
        pages[0] = Page::Ready(entries(&data[HEADER..], count.min(page)));
        Ok(Directory {
            min: lo,
            max: hi,
            count,
            page,
            pages,
        })
    }

    /// Entries in page `p`.
    fn page_entries(&self, p: usize) -> usize {
        self.page.min(self.count - p * self.page)
    }

    /// Byte range (start, length) of directory page `p` in the pack.
    pub fn page_range(&self, p: usize) -> (u32, u32) {
        let start = HEADER + ENTRY * p * self.page;
        (start as u32, (ENTRY * self.page_entries(p)) as u32)
    }

    /// Page `p` was requested.
    pub fn page_loading(&mut self, p: usize) {
        if let Some(slot @ Page::Missing) = self.pages.get_mut(p) {
            *slot = Page::Loading;
        }
    }

    /// Page `p` arrived (exactly its [`Self::page_range`] bytes) or failed;
    /// the tiles it lists are absent after a failure.
    pub fn page_loaded(&mut self, p: usize, data: Result<&[u8], String>) -> Result<(), String> {
        let Some(n) = (p < self.pages.len()).then(|| self.page_entries(p)) else {
            return Err(format!("tile pack: no directory page {p}"));
        };
        let (page, result) = match data {
            Ok(d) if d.len() == n * ENTRY => (Page::Ready(entries(d, n)), Ok(())),
            Ok(d) => (
                Page::Failed,
                Err(format!(
                    "tile pack: directory page {p} has {} bytes",
                    d.len()
                )),
            ),
            Err(e) => (Page::Failed, Err(e)),
        };
        self.pages[p] = page;
        result
    }

    /// Where tile `t` is.
    pub fn range(&self, t: TileId) -> Lookup {
        if t.z < self.min || t.z > self.max {
            return Lookup::Absent;
        }
        let (cols, rows) = grid(t.z);
        if t.x >= cols || t.y >= rows {
            return Lookup::Absent;
        }
        let i = tile_count(self.min, t.z) - (cols * rows) as usize + (t.y * cols + t.x) as usize;
        let p = i / self.page;
        match &self.pages[p] {
            Page::Ready(e) => match e[i % self.page] {
                (offset, len) if len > 0 => Lookup::Tile(offset, len),
                _ => Lookup::Absent,
            },
            Page::Missing => Lookup::NeedPage(p),
            Page::Loading => Lookup::Waiting,
            Page::Failed => Lookup::Absent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TINY: &[u8] = include_bytes!("../scripts/testdata/tiny.tiles");

    fn id(z: u8, x: u32, y: u32) -> TileId {
        TileId { z, x, y }
    }

    fn bytes_of(dir: &Directory, t: TileId) -> Option<&'static [u8]> {
        match dir.range(t) {
            Lookup::Tile(o, l) => Some(&TINY[o as usize..(o + l) as usize]),
            _ => None,
        }
    }

    #[test]
    fn parses_the_shared_fixture() {
        let dir = Directory::parse(TINY, 0, 1, DIR_PAGE).unwrap();
        assert_eq!((dir.min, dir.max), (0, 1));
        assert_eq!(header_len(0, 1), 96);
        assert_eq!(first_len(0, 1, DIR_PAGE), 96);
        assert_eq!(bytes_of(&dir, id(0, 1, 0)), Some(&b"z0x1y0"[..]));
        assert_eq!(bytes_of(&dir, id(1, 2, 1)), Some(&b"z1x2y1"[..]));
        assert_eq!(bytes_of(&dir, id(1, 0, 0)), Some(&b"z1x0y0"[..]));
        // Absent in the fixture, and outside the pyramid.
        assert_eq!(dir.range(id(1, 3, 1)), Lookup::Absent);
        assert_eq!(dir.range(id(2, 0, 0)), Lookup::Absent);
        assert_eq!(dir.range(id(1, 4, 0)), Lookup::Absent);
        assert_eq!(dir.range(id(0, 0, 1)), Lookup::Absent);
        // Only the header and directory are needed.
        assert!(Directory::parse(&TINY[..96], 0, 1, DIR_PAGE).is_ok());
    }

    #[test]
    fn loads_the_directory_in_pages() {
        // Pages of 4 entries: level 0 (2) and level 1 (8) make pages of
        // 4, 4 and 2 entries; the first fetch holds page 0.
        assert_eq!(first_len(0, 1, 4), 16 + 32);
        let mut dir = Directory::parse(&TINY[..48], 0, 1, 4).unwrap();
        assert_eq!(bytes_of(&dir, id(0, 1, 0)), Some(&b"z0x1y0"[..]));
        assert_eq!(bytes_of(&dir, id(1, 1, 0)), Some(&b"z1x1y0"[..]), "entry 3");
        assert_eq!(dir.range(id(1, 2, 0)), Lookup::NeedPage(1));
        assert_eq!(dir.range(id(1, 2, 1)), Lookup::NeedPage(2));
        assert_eq!(dir.page_range(1), (48, 32));
        assert_eq!(dir.page_range(2), (80, 16));
        dir.page_loading(1);
        assert_eq!(dir.range(id(1, 2, 0)), Lookup::Waiting);
        dir.page_loaded(1, Ok(&TINY[48..80])).unwrap();
        assert_eq!(bytes_of(&dir, id(1, 2, 0)), Some(&b"z1x2y0"[..]));
        assert_eq!(
            bytes_of(&dir, id(1, 1, 1)),
            Some(&b"z1x1y1"[..]),
            "entry 7, also page 1"
        );
        // A short page fails, and its tiles count as absent.
        assert!(dir.page_loaded(2, Ok(&TINY[80..88])).is_err());
        assert_eq!(dir.range(id(1, 2, 1)), Lookup::Absent);
        assert!(dir.page_loaded(3, Ok(&[])).is_err());
        dir.page_loaded(2, Ok(&TINY[80..96])).unwrap();
        assert_eq!(bytes_of(&dir, id(1, 2, 1)), Some(&b"z1x2y1"[..]));
        assert_eq!(dir.range(id(1, 3, 1)), Lookup::Absent);
        assert!(Directory::parse(&TINY[..47], 0, 1, 4).is_err());
        assert!(Directory::parse(TINY, 0, 1, 0).is_err());
    }

    #[test]
    fn deep_packs_read_little_before_the_first_tile() {
        // Levels 3..7: 43648 entries, 349 KB; the first fetch is 32 KB and
        // covers every tile of levels 3..5.
        assert_eq!(header_len(3, 7), 16 + 8 * 43648);
        assert_eq!(first_len(3, 7, DIR_PAGE), 16 + 8 * DIR_PAGE);
        assert!(tile_count(3, 5) <= DIR_PAGE);
    }

    #[test]
    fn counts_match_the_python_writer() {
        assert_eq!(tile_count(3, 5), 128 + 512 + 2048);
        assert_eq!(header_len(3, 5), 16 + 8 * 2688);
        assert_eq!(tile_count(0, 0), 2);
    }

    #[test]
    fn rejects_bad_headers() {
        let mut bad = TINY.to_vec();
        bad[0] = b'X';
        assert!(Directory::parse(&bad, 0, 1, DIR_PAGE)
            .unwrap_err()
            .contains("magic"));
        let mut bad = TINY.to_vec();
        bad[4] = 2;
        assert!(Directory::parse(&bad, 0, 1, DIR_PAGE)
            .unwrap_err()
            .contains("version"));
        let mut bad = TINY.to_vec();
        bad[7] = 9;
        assert!(Directory::parse(&bad, 0, 1, DIR_PAGE)
            .unwrap_err()
            .contains("format"));
        let mut bad = TINY.to_vec();
        bad[10] = 0;
        assert!(Directory::parse(&bad, 0, 1, DIR_PAGE)
            .unwrap_err()
            .contains("border"));
        let mut bad = TINY.to_vec();
        bad[12] = 11;
        assert!(Directory::parse(&bad, 0, 1, DIR_PAGE)
            .unwrap_err()
            .contains("tiles"));
        assert!(Directory::parse(TINY, 0, 2, DIR_PAGE)
            .unwrap_err()
            .contains("levels"));
        assert!(Directory::parse(TINY, 1, 1, DIR_PAGE)
            .unwrap_err()
            .contains("levels"));
        assert!(Directory::parse(&TINY[..40], 0, 1, DIR_PAGE)
            .unwrap_err()
            .contains("truncated"));
        assert!(Directory::parse(&TINY[..8], 0, 1, DIR_PAGE)
            .unwrap_err()
            .contains("truncated"));
        assert!(Directory::parse(&[], 0, 1, DIR_PAGE).is_err());
    }
}
