#!/usr/bin/env python3
"""Tests for convert_constellations.py. `--write-fixture` rewrites
scripts/testdata/tiny_constellations.bin, which src/constellations.rs also
decodes."""
import io
import math
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import convert_constellations as cc  # noqa: E402

FIXTURE = Path(__file__).parent / "testdata" / "tiny_constellations.bin"

CSV = """\
"id","hip","ra","dec"
1,10,0.0,0.0
2,11,1.0,10.0
3,12,2.0,0.0
4,20,23.5,-60.0
5,21,0.5,-60.0
6,,5.0,5.0
"""

INDEX = {
    "constellations": [
        {"iau": "Zzz", "common_name": {"native": "Zeta"}, "lines": [[20, 21]]},
        {"iau": "Abc", "common_name": {"native": "Alpha"}, "lines": [[10, 11, 12], ["thin", 11, 12]]},
    ]
}


def figs():
    return cc.figures(INDEX, cc.hip_positions(io.StringIO(CSV)))


def fixture_bytes():
    return cc.encode(figs())


class Convert(unittest.TestCase):
    def test_round_trip(self):
        out = cc.decode(fixture_bytes())
        self.assertEqual([f[0] for f in out], ["Abc", "Zzz"], "sorted by abbreviation")
        abc = out[0]
        self.assertEqual(abc[1], "Alpha")
        self.assertEqual([l[0] for l in abc[3]], [0, cc.THIN])
        ra, dec = abc[3][0][1][1]
        self.assertAlmostEqual(math.degrees(ra), 15.0, places=6)
        self.assertAlmostEqual(math.degrees(dec), 10.0, places=6)

    def test_label_is_the_mean_direction(self):
        zzz = [f for f in figs() if f[0] == "Zzz"][0]
        ra, dec = zzz[2]
        self.assertAlmostEqual(ra, 0.0, places=6, msg="across 0h: 23.5h and 0.5h average to 0h")
        self.assertLess(dec, -60.0, "the mean of two points on a small circle is poleward")
        abc = [f for f in figs() if f[0] == "Abc"][0]
        self.assertAlmostEqual(abc[2][0], 15.0, places=6)

    def test_deterministic(self):
        rev = {"constellations": INDEX["constellations"][::-1]}
        self.assertEqual(cc.encode(cc.figures(rev, cc.hip_positions(io.StringIO(CSV)))), fixture_bytes())

    def test_rejects_bad_input(self):
        pos = cc.hip_positions(io.StringIO(CSV))
        for bad in (
            [[10, 99]],          # unknown star
            [[10]],              # one point
            [["thin"]],          # flag only
            [[10, "x"]],         # not a number
            [],                  # no lines
        ):
            idx = {"constellations": [{"iau": "Abc", "common_name": {"native": "A"}, "lines": bad}]}
            with self.assertRaises(ValueError, msg=bad):
                cc.figures(idx, pos)
        dup = {"constellations": INDEX["constellations"] + INDEX["constellations"][:1]}
        with self.assertRaises(ValueError):
            cc.figures(dup, pos)

    def test_rejects_bad_files(self):
        good = fixture_bytes()
        for bad in (b"XNST" + good[4:], good[:-1], good + b"\0", good[:13]):
            with self.assertRaises(ValueError):
                cc.decode(bad)

    def test_committed_fixture_is_current(self):
        self.assertEqual(FIXTURE.read_bytes(), fixture_bytes(),
                         "run: python3 scripts/test_convert_constellations.py --write-fixture")


if __name__ == "__main__":
    if sys.argv[1:] == ["--write-fixture"]:
        FIXTURE.write_bytes(fixture_bytes())
        print(f"wrote {FIXTURE}")
    else:
        unittest.main()
