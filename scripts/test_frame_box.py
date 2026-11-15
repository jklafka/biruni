#!/usr/bin/env python3
"""Unit tests for the country framing in convert_geojson.py (stdlib only).

Run: python3 scripts/test_frame_box.py (scripts/check-data.sh runs it too).
"""
import importlib.util
import json
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "convert_geojson", Path(__file__).with_name("convert_geojson.py")
)
cg = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cg)
frame = cg._frame_box


class FrameBox(unittest.TestCase):
    def test_single_polygon(self):
        self.assertEqual(frame([(10, 20, 30, 40)]), (20, 20, 40, 20))

    def test_archipelago_is_framed_whole(self):
        # Each island is within the link distance of the next: all included.
        c, _, _, w = frame([(100, 0, 110, 5), (115, -3, 120, 2), (125, 0, 130, 4)])
        self.assertEqual((c, w), (115, 30))

    def test_far_overseas_part_is_dropped(self):
        mainland = (-5, 42, 8, 51)
        guiana = (-54, 2, -51, 6)
        self.assertEqual(frame([guiana, mainland]), (1.5, 42, 51, 13))

    def test_seam_straddler_is_narrow(self):
        # Fiji-like: polygons on both sides of the 180th meridian.
        c, _, _, w = frame([(177, -19, 180, -16), (-180, -17, -178, -16), (178, -18, 179, -17)])
        self.assertAlmostEqual(w, 5)
        self.assertAlmostEqual(c, 179.5)

    def test_seam_center_is_on_the_seam(self):
        c, _, _, w = frame([(176, 0, 180, 1), (-180, 0, -176, 1)])
        self.assertAlmostEqual(w, 8)
        self.assertAlmostEqual(abs(c), 180.0)
        self.assertTrue(-180 <= c < 180)

    def test_wide_country_across_seam(self):
        # Russia-like: the frame runs east past 180 instead of back around.
        c, _, _, w = frame([(20, 50, 180, 75), (-180, 65, -169, 70)])
        self.assertAlmostEqual(w, 171)
        self.assertAlmostEqual(c, 20 + 171 / 2)

    def test_islands_linked_across_seam(self):
        _, _, _, w = frame([(172, 0, 174, 1), (-176, 0, -174, 1)])
        self.assertAlmostEqual(w, 14)

    def test_full_circle(self):
        boxes = [(-180, 0, -100, 10), (-100, 0, 0, 10), (0, 0, 100, 10), (100, 0, 180, 10)]
        self.assertAlmostEqual(frame(boxes)[3], 360.0)

    def test_poles(self):
        c, lat0, lat1, w = frame([(-180, -90, 180, -60)])
        self.assertEqual((lat0, lat1), (-90, -60))
        self.assertAlmostEqual(w, 360)
        self.assertTrue(-180 <= c < 180)

    def test_point_polygon(self):
        self.assertEqual(frame([(5, 5, 5, 5)]), (5, 5, 5, 0))

    def test_lon_gap(self):
        self.assertEqual(cg._lon_gap((0, 10), (5, 20)), 0)
        self.assertEqual(cg._lon_gap((170, 175), (-175, -170)), 10)
        self.assertEqual(cg._lon_gap((0, 10), (30, 40)), 20)

    def test_polygon_boxes(self):
        poly = [[[0, 0], [4, 0], [4, 3], [0, 0]]]
        self.assertEqual(cg._polygon_boxes({"type": "Polygon", "coordinates": poly}), [(0, 0, 4, 3)])
        multi = {"type": "MultiPolygon", "coordinates": [poly, poly]}
        self.assertEqual(len(cg._polygon_boxes(multi)), 2)

    def test_bundled_countries(self):
        src = Path(__file__).resolve().parent.parent / "data-src" / cg.COUNTRIES_SRC
        by_name = {}
        for f in json.loads(src.read_text(encoding="utf-8"))["features"]:
            if f.get("geometry"):
                by_name[f["properties"]["NAME"]] = frame(cg._polygon_boxes(f["geometry"]))
        c, _, _, w = by_name["Fiji"]
        self.assertLess(w, 12)
        self.assertGreater(abs(c), 170)
        self.assertLess(by_name["France"][3], 25)  # no French Guiana or Pacific territories
        c, _, _, w = by_name["Indonesia"]
        self.assertTrue(90 < c < 140 and w > 40)
        self.assertGreater(by_name["Kiribati"][3], 30)
        for name, (c, lat0, lat1, w) in by_name.items():
            self.assertTrue(-180 <= c < 180 and 0 <= w <= 360 and lat0 <= lat1, name)


if __name__ == "__main__":
    unittest.main()
