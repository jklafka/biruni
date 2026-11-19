//! Tile pack header and directory (browser-free).
//!
//! A tile pack holds one layer's equirectangular tile pyramid; the format is
//! described in `docs/tile-streaming.md` and written by
//! `scripts/tile_pack.py`. Both parse `scripts/testdata/tiny.tiles` in their
//! tests, so the two stay in step. The app fetches the header and directory
//! with one range request ([`header_len`]), then each tile by its byte range.

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

/// Bytes of the header plus directory.
pub fn header_len(min: u8, max: u8) -> usize {
    HEADER + ENTRY * tile_count(min, max)
}

/// The parsed directory: where each tile's bytes are in the pack. The tile
/// size field is not checked here (the fixture's tiles are tiny); the
/// renderer checks each decoded tile.
#[derive(Debug)]
pub struct Directory {
    pub min: u8,
    pub max: u8,
    entries: Vec<(u32, u32)>,
}

impl Directory {
    /// Parse the start of a pack (at least [`header_len`] bytes) and check it
    /// against the levels that `tilesets.json` promised.
    pub fn parse(data: &[u8], min: u8, max: u8) -> Result<Directory, String> {
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
        if data.len() < header_len(lo, hi) {
            return Err("tile pack: truncated directory".into());
        }
        let u32_at =
            |i: usize| u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        let entries = (0..count)
            .map(|i| {
                let at = HEADER + i * ENTRY;
                (u32_at(at), u32_at(at + 4))
            })
            .collect();
        Ok(Directory {
            min: lo,
            max: hi,
            entries,
        })
    }

    /// Byte range (start, length) of a tile, or `None` if the pack lacks it
    /// or it is outside the pyramid.
    pub fn range(&self, t: TileId) -> Option<(u32, u32)> {
        if t.z < self.min || t.z > self.max {
            return None;
        }
        let (cols, rows) = grid(t.z);
        if t.x >= cols || t.y >= rows {
            return None;
        }
        let i = tile_count(self.min, t.z) - (cols * rows) as usize + (t.y * cols + t.x) as usize;
        let (offset, len) = self.entries[i];
        (len > 0).then_some((offset, len))
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
        dir.range(t)
            .map(|(o, l)| &TINY[o as usize..(o + l) as usize])
    }

    #[test]
    fn parses_the_shared_fixture() {
        let dir = Directory::parse(TINY, 0, 1).unwrap();
        assert_eq!((dir.min, dir.max), (0, 1));
        assert_eq!(header_len(0, 1), 96);
        assert_eq!(bytes_of(&dir, id(0, 1, 0)), Some(&b"z0x1y0"[..]));
        assert_eq!(bytes_of(&dir, id(1, 2, 1)), Some(&b"z1x2y1"[..]));
        assert_eq!(bytes_of(&dir, id(1, 0, 0)), Some(&b"z1x0y0"[..]));
        // Absent in the fixture, and outside the pyramid.
        assert_eq!(dir.range(id(1, 3, 1)), None);
        assert_eq!(dir.range(id(2, 0, 0)), None);
        assert_eq!(dir.range(id(1, 4, 0)), None);
        assert_eq!(dir.range(id(0, 0, 1)), None);
        // Only the header and directory are needed.
        assert!(Directory::parse(&TINY[..96], 0, 1).is_ok());
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
        assert!(Directory::parse(&bad, 0, 1).unwrap_err().contains("magic"));
        let mut bad = TINY.to_vec();
        bad[4] = 2;
        assert!(Directory::parse(&bad, 0, 1)
            .unwrap_err()
            .contains("version"));
        let mut bad = TINY.to_vec();
        bad[7] = 9;
        assert!(Directory::parse(&bad, 0, 1).unwrap_err().contains("format"));
        let mut bad = TINY.to_vec();
        bad[10] = 0;
        assert!(Directory::parse(&bad, 0, 1).unwrap_err().contains("border"));
        let mut bad = TINY.to_vec();
        bad[12] = 11;
        assert!(Directory::parse(&bad, 0, 1).unwrap_err().contains("tiles"));
        assert!(Directory::parse(TINY, 0, 2).unwrap_err().contains("levels"));
        assert!(Directory::parse(TINY, 1, 1).unwrap_err().contains("levels"));
        assert!(Directory::parse(&TINY[..40], 0, 1)
            .unwrap_err()
            .contains("truncated"));
        assert!(Directory::parse(&TINY[..8], 0, 1)
            .unwrap_err()
            .contains("truncated"));
        assert!(Directory::parse(&[], 0, 1).is_err());
    }
}
