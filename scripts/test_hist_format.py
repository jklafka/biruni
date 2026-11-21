#!/usr/bin/env python3
"""Tests for hist_format.py. `--write-fixture` rewrites
scripts/testdata/tiny_history.bin, which src/history.rs also decodes."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import hist_format as hf  # noqa: E402

FIXTURE = Path(__file__).parent / "testdata" / "tiny_history.bin"

SQUARE = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
HOLE = [(4.0, 4.0), (6.0, 4.0), (6.0, 6.0), (4.0, 6.0)]
WEST = [(-20.0, 0.0), (-10.0, 0.0), (-10.0, 10.0), (-20.0, 10.0)]
SEAM = [(170.0, -5.0), (180.0, -5.0), (180.0, 5.0), (170.0, 5.0)]


def fixture():
    """Three polities: 'Squareland' from 44 BCE to 99 CE (with a hole), then
    with the western square added from 100 to 199; 'Ost' 150 to 2024 on the
    seam; and 'Ünïcode' for a single year, 1 BCE."""
    names = ["Squareland", "Ost", "Ünïcode"]
    geoms = [[SQUARE, HOLE], [SQUARE, HOLE, WEST], [SEAM]]
    records = [
        (0, -44, 99, 0, (2.0, 8.0), 0),
        (0, 100, 199, 1, (2.0, 8.0), 0),
        (1, 150, 2024, 2, (175.0, 0.0), 7),
        (2, -1, -1, 2, (175.0, 1.0), 0),
    ]
    return hf.SCALE_YEARS, 3, names, geoms, records


class Format(unittest.TestCase):
    def test_round_trip(self):
        scale, digits, names, geoms, records = fixture()
        data = hf.encode(scale, digits, names, geoms, records)
        self.assertEqual(hf.decode(data), (scale, digits, names, geoms, records))

    def test_rings_are_shared(self):
        scale, digits, names, geoms, records = fixture()
        data = hf.encode(scale, digits, names, geoms, records)
        # SQUARE and HOLE appear in two geometries but are stored once:
        # dropping the second copy of each from the input changes nothing.
        once = hf.encode(scale, digits, names, [geoms[0], geoms[0] + [WEST], geoms[2]], records)
        self.assertEqual(data, once)
        r = hf.Reader(data, hf.HEADER.size)
        for _ in range(r.varint()):
            r.take(r.varint())
        self.assertEqual(r.varint(), 4, "four distinct rings")

    def test_quantization(self):
        data = hf.encode(hf.SCALE_MA, 2, ["a"], [[[(0.123, 0.456), (1.0, 0.0), (0.0, 1.0)]]],
                         [(0, -5400, 0, 0, (0.0, 0.0), 801)])
        scale, digits, _, geoms, records = hf.decode(data)
        self.assertEqual((scale, digits), (hf.SCALE_MA, 2))
        self.assertEqual(geoms[0][0][0], (0.12, 0.46))
        self.assertEqual(records[0][1:3], (-5400, 0))
        self.assertEqual(records[0][5], 801)

    def test_bad_input(self):
        data = hf.encode(*fixture())
        for bad in (b"", data[:-1], data + b"\0", b"HISX" + data[4:], data[:4] + b"\x02" + data[5:],
                    data[:5] + b"\x05" + data[6:]):
            with self.assertRaises(ValueError):
                hf.decode(bad)
        with self.assertRaises(ValueError):
            hf.encode(0, 3, ["a"], [[[(0, 0), (1, 1)]]], [])
        with self.assertRaises(ValueError):
            hf.encode(0, 3, ["a"], [[SQUARE]], [(0, 5, 4, 0, (0, 0), 0)])
        with self.assertRaises(ValueError):
            hf.encode(0, 3, ["a"], [[SQUARE]], [(1, 0, 4, 0, (0, 0), 0)])

    def test_out_of_range(self):
        data = hf.encode(0, 3, ["a"], [[[(179.9, 0), (180.0, 0), (180.0, 1)]]], [(0, 0, 0, 0, (0, 0), 0)])
        hf.decode(data)
        with self.assertRaises(ValueError):
            hf.decode(hf.encode(0, 3, ["a"], [[[(181.0, 0), (180.0, 0), (180.0, 1)]]], [(0, 0, 0, 0, (0, 0), 0)]))

    def test_fixture_is_current(self):
        self.assertEqual(FIXTURE.read_bytes(), hf.encode(*fixture()),
                         "run: python3 scripts/test_hist_format.py --write-fixture")


if __name__ == "__main__":
    if sys.argv[1:] == ["--write-fixture"]:
        FIXTURE.write_bytes(hf.encode(*fixture()))
        print(f"wrote {FIXTURE}")
    else:
        unittest.main()
