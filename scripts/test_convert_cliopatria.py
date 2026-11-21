#!/usr/bin/env python3
"""Tests for convert_cliopatria.py on small made-up features."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import convert_cliopatria as cc  # noqa: E402
import hist_format as hf  # noqa: E402


def square(x0, y0, size, n=40):
    """A square ring with `n` points per side (so simplification has work)."""
    pts = []
    for i in range(n):
        pts.append((x0 + size * i / n, y0))
    for i in range(n):
        pts.append((x0 + size, y0 + size * i / n))
    for i in range(n):
        pts.append((x0 + size - size * i / n, y0 + size))
    for i in range(n):
        pts.append((x0, y0 + size - size * i / n))
    return pts + [pts[0]]


def feature(name, start, end, rings, kind="POLITY", area=1e6):
    return {
        "type": "Feature",
        "properties": {"Name": name, "FromYear": start, "ToYear": end, "Type": kind, "Area": area},
        "geometry": {"type": "MultiPolygon", "coordinates": [[[list(p) for p in r]] for r in rings]},
    }


class Convert(unittest.TestCase):
    def setUp(self):
        self.features = [
            feature("West", -100, 100, [square(0, 0, 10)], area=6e6),
            feature("East", -50, 50, [square(10, 0, 10)], area=2e4),
            # West changes shape at 101 but keeps its island.
            feature("West", 101, 300, [square(0, 0, 12), square(30, 30, 1)], area=6e6),
            feature("Union", -100, 100, [square(0, 0, 20)], kind="RELATION"),
            feature("Speck", 0, 10, [square(50, 50, 0.001)]),
        ]
        self.data = hf.decode(cc.convert(self.features))

    def test_records_and_names(self):
        scale, digits, names, geoms, records = self.data
        self.assertEqual(scale, hf.SCALE_YEARS)
        self.assertEqual(digits, cc.DIGITS)
        self.assertNotIn("Union", names, "relations are left out")
        spans = sorted((names[r[0]], r[1], r[2]) for r in records)
        self.assertEqual(spans, [("East", -50, 50), ("West", -100, 100), ("West", 101, 300)])

    def test_simplified_and_inside(self):
        _, _, names, geoms, records = self.data
        for r in records:
            for ring in geoms[r[3]]:
                self.assertLessEqual(len(ring), 6, "a square needs about 4 points")
            self.assertTrue(cc.inside(geoms[r[3]], *r[4]), "label inside")

    def test_neighbors_differ_and_tiers(self):
        _, _, names, _, records = self.data
        color = {names[r[0]]: r[5] & 7 for r in records}
        self.assertNotEqual(color["West"], color["East"])
        tier = {names[r[0]]: r[5] >> 3 for r in records}
        self.assertEqual(tier["West"], 0)
        self.assertEqual(tier["East"], 5)

    def test_rings_are_shared(self):
        # The same input ring twice is stored once.
        a = cc.convert(self.features)
        b = cc.convert(self.features + [feature("Copy", 400, 500, [square(30, 30, 1)])])
        self.assertLess(len(b) - len(a), 30)

    def test_deterministic(self):
        self.assertEqual(cc.convert(self.features), cc.convert(list(reversed(self.features))))


if __name__ == "__main__":
    unittest.main()
