#!/usr/bin/env python3
"""Tests for convert_stars.py. `--write-fixture` rewrites
scripts/testdata/tiny_stars.bin, which src/stars.rs also decodes."""
import io
import math
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import convert_stars as cs  # noqa: E402

FIXTURE = Path(__file__).parent / "testdata" / "tiny_stars.bin"

CSV = """\
"id","hip","proper","ra","dec","mag","ci"
0,,Sol,0.0,0.0,-26.7,0.656
1,1,"",0.00006,1.089009,7.1,0.482
2,2,Vega,18.615649,38.783692,0.03,-0.001
3,3,"",23.999999,-89.5,5.0,
4,4,Faint,12.0,0.0,8.5,1.0
5,5,"",6.0,-45.0,0.03,2.9
6,6,"",,10.0,3.0,0.1
"""


def rows():
    return cs.read_rows(io.StringIO(CSV))


def fixture_bytes():
    return cs.encode(rows())


class Convert(unittest.TestCase):
    def test_reads_only_usable_stars(self):
        r = rows()
        self.assertEqual(len(r), 4, "Sun, too faint and no RA are dropped")
        self.assertNotIn("Sol", [x[4] for x in r])
        self.assertIsNone([x for x in r if x[2] == 5.0][0][3], "unknown color index")

    def test_round_trip_and_order(self):
        stars = cs.decode(fixture_bytes())
        self.assertEqual([s[2] for s in stars], sorted(s[2] for s in stars), "brightest first")
        vega = [s for s in stars if s[4] == "Vega"][0]
        self.assertAlmostEqual(math.degrees(vega[0]) / 15.0, 18.615649, places=6)
        self.assertAlmostEqual(math.degrees(vega[1]), 38.783692, places=6)
        self.assertAlmostEqual(vega[2], 0.05, places=9, msg="0.03 rounds to the 0.05 step")
        self.assertAlmostEqual(vega[3], 0.0)
        # Equal magnitudes: ordered by right ascension (6h before 18.6h).
        self.assertEqual([s[4] for s in stars[:2]], ["", "Vega"])
        south = [s for s in stars if s[2] == 5.0][0]
        self.assertIsNone(south[3])
        self.assertAlmostEqual(math.degrees(south[1]), -89.5, places=6)
        self.assertAlmostEqual(math.degrees(south[0]) / 15.0, 23.999999, places=5)
        red = stars[0]
        self.assertAlmostEqual(red[3], 2.54, msg="clamped to the i8 range")

    def test_deterministic(self):
        shuffled = CSV.splitlines()
        shuffled = "\n".join([shuffled[0]] + shuffled[1:][::-1]) + "\n"
        self.assertEqual(cs.encode(cs.read_rows(io.StringIO(shuffled))), fixture_bytes())

    def test_rejects_bad_files(self):
        good = fixture_bytes()
        for bad in (b"XTAR" + good[4:], good[:20], good + b"x", good[:-1]):
            with self.assertRaises(ValueError):
                cs.decode(bad)

    def test_committed_fixture_is_current(self):
        self.assertEqual(FIXTURE.read_bytes(), fixture_bytes(),
                         "run: python3 scripts/test_convert_stars.py --write-fixture")


if __name__ == "__main__":
    if sys.argv[1:] == ["--write-fixture"]:
        FIXTURE.write_bytes(fixture_bytes())
        print(f"wrote {FIXTURE}")
    else:
        unittest.main()
