#!/usr/bin/env python3
"""Unit tests for scripts/tile_pack.py (stdlib only).

Run: python3 scripts/test_tile_pack.py (scripts/check-data.sh runs it too).
"""
import struct
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import tile_pack  # noqa: E402

# Shared with src/tile_pack.rs, which parses the same bytes.
FIXTURE = Path(__file__).parent / "testdata" / "tiny.tiles"


def fixture_tiles():
    """Levels 0..1 (2 + 8 tiles); tile (1, 3, 1) is absent."""
    tiles = {}
    for z in (0, 1):
        cols, rows = tile_pack.grid(z)
        for y in range(rows):
            for x in range(cols):
                if (z, x, y) != (1, 3, 1):
                    tiles[(z, x, y)] = f"z{z}x{x}y{y}".encode()
    return tiles


class TilePackTest(unittest.TestCase):
    def test_grid_and_counts(self):
        self.assertEqual(tile_pack.grid(0), (2, 1))
        self.assertEqual(tile_pack.grid(5), (64, 32))
        self.assertEqual(tile_pack.tile_count(3, 5), 128 + 512 + 2048)
        self.assertEqual(tile_pack.header_len(3, 5), 16 + 8 * 2688)
        self.assertEqual(tile_pack.index(3, 3, 0, 0), 0)
        self.assertEqual(tile_pack.index(3, 4, 0, 0), 128)
        self.assertEqual(tile_pack.index(3, 4, 31, 15), 128 + 511)
        with self.assertRaises(tile_pack.PackError):
            tile_pack.index(3, 4, 32, 0)

    def test_round_trip_and_fixture(self):
        data = tile_pack.write(0, 1, fixture_tiles(), tile_size=4)
        self.assertEqual(data, FIXTURE.read_bytes(), "regenerate scripts/testdata/tiny.tiles with --write-fixture if the format changed")
        info = tile_pack.check(data)
        self.assertEqual(info, {"min_level": 0, "max_level": 1, "format": "webp",
                                "tile_size": 4, "border": 1, "count": 10, "present": 9})
        self.assertEqual(tile_pack.tile(data, 0, 1, 2, 1), b"z1x2y1")
        self.assertEqual(tile_pack.tile(data, 0, 0, 1, 0), b"z0x1y0")
        self.assertIsNone(tile_pack.tile(data, 0, 1, 3, 1))
        # Tile bytes follow the directory in directory order, so a row is contiguous.
        _, entries = tile_pack.read_header(data)
        self.assertEqual(entries[0][0], tile_pack.header_len(0, 1))
        self.assertEqual(entries[2][0] + entries[2][1], entries[3][0])

    def test_rejects_bad_packs(self):
        good = tile_pack.write(0, 1, fixture_tiles(), tile_size=4)
        bad = {
            "magic": b"XTIL" + good[4:],
            "version": good[:4] + b"\x02" + good[5:],
            "levels": good[:5] + b"\x02\x01" + good[7:],
            "format": good[:7] + b"\x09" + good[8:],
            "count": good[:12] + struct.pack("<I", 11) + good[16:],
            "truncated directory": good[:40],
            "truncated header": good[:10],
        }
        for what, data in bad.items():
            with self.subTest(what), self.assertRaises(tile_pack.PackError):
                tile_pack.check(data)
        with self.assertRaises(tile_pack.PackError):
            tile_pack.check(good[:-1])  # last tile runs past the end
        overlap = bytearray(good)
        struct.pack_into("<I", overlap, 16 + 8, 16)  # tile 1 starts inside the directory
        with self.assertRaises(tile_pack.PackError):
            tile_pack.check(bytes(overlap))
        with self.assertRaises(tile_pack.PackError):
            tile_pack.write(0, 0, {(1, 0, 0): b"x"})

    def test_sparse_pack(self):
        data = tile_pack.write(3, 5, {(5, 33, 9): b"only"})
        info = tile_pack.check(data)
        self.assertEqual(info["present"], 1)
        self.assertEqual(tile_pack.tile(data, 3, 5, 33, 9), b"only")
        self.assertEqual(len(data), tile_pack.header_len(3, 5) + 4)


try:
    import PIL  # noqa: F401
    HAVE_PIL = True
except ImportError:
    HAVE_PIL = False


@unittest.skipUnless(HAVE_PIL, "make_tiles.py needs Pillow")
class Padding(unittest.TestCase):
    def test_border_wraps_longitude_and_repeats_the_poles(self):
        import make_tiles
        from PIL import Image

        im = Image.new("RGB", (8, 4))
        px = im.load()
        for x in range(8):
            for y in range(4):
                px[x, y] = (x * 30, y * 60, 0)
        p = make_tiles.padded(im, 1)
        q = p.load()
        self.assertEqual(p.size, (10, 6))
        self.assertEqual(q[0, 1], px[7, 0], "west border is the east edge")
        self.assertEqual(q[9, 2], px[0, 1], "east border is the west edge")
        self.assertEqual(q[3, 0], px[2, 0], "north border repeats the top row")
        self.assertEqual(q[3, 5], px[2, 3], "south border repeats the bottom row")
        self.assertEqual((q[0, 0], q[9, 5]), (px[7, 0], px[0, 3]))
        self.assertEqual(q[4, 2], px[3, 1], "interior shifted by the border")


if __name__ == "__main__":
    if sys.argv[1:] == ["--write-fixture"]:
        FIXTURE.write_bytes(tile_pack.write(0, 1, fixture_tiles(), tile_size=4))
        print(f"wrote {FIXTURE}")
    else:
        unittest.main()
