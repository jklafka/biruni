#!/usr/bin/env python3
"""Tests for make_population_density.py on made-up grids (no download).
Needs NumPy; skipped without it (CI's data job does not install it)."""
import math
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

try:
    import numpy as np
except ImportError:  # pragma: no cover
    np = None

import make_population_density as m  # noqa: E402


@unittest.skipIf(np is None, "needs NumPy")
class PopulationDensityTest(unittest.TestCase):
    def test_codes(self):
        c = m.code([0.0, 0.05, m.MIN_DENSITY, 1.0, m.MAX_DENSITY, 1e9, float("nan"), -3.0])
        self.assertEqual(list(c[:3]), [0, 0, 1])
        self.assertEqual(c[4], 255)
        self.assertEqual(c[5], 255)
        self.assertEqual(c[6], 0, "no data")
        self.assertEqual(c[7], 0)
        t = (0 - math.log10(m.MIN_DENSITY)) / (math.log10(m.MAX_DENSITY) - math.log10(m.MIN_DENSITY))
        self.assertEqual(c[3], 1 + round(t * 254))

    def test_cell_areas_cover_the_sphere(self):
        a = m.cell_areas_km2(180, 360)
        total = a.sum() * 360
        self.assertAlmostEqual(total / (4 * math.pi * m.EARTH_RADIUS_KM**2), 1.0, places=9)
        self.assertGreater(a[90], a[0] * 50, "equator cells are larger than polar ones")

    def test_accumulate_sums_people_into_cells(self):
        # A 1-degree source grid from 180 W, 90 N, in two tiles, into a
        # 4x2 output (90 degree cells).
        step = 1.0
        left = np.zeros((180, 180))
        right = np.zeros((180, 180))
        left[10, 10] = 5.0  # 79.5 N, 169.5 W -> row 0, col 0
        left[100, 100] = 7.0  # 9.5 S, 79.5 W -> row 1, col 1
        right[0, 179] = 2.0  # 89.5 N, 179.5 E -> row 0, col 3
        right[5, 5] = float("nan")
        right[6, 6] = -200.0
        out = m.accumulate([(left, 0, 0), (right, 0, 180)], (-180.0, 90.0), step, (2, 4))
        self.assertEqual(out.tolist(), [[5.0, 0.0, 0.0, 2.0], [0.0, 7.0, 0.0, 0.0]])

    def test_accumulate_wraps_longitudes_past_the_seam(self):
        # The real grid starts a hair west of 180 W.
        tile = np.zeros((2, 2))
        tile[0, 0] = 3.0
        out = m.accumulate([(tile, 0, 0)], (-180.5, 90.0), 0.25, (2, 4))
        self.assertEqual(out[0, 3], 3.0, "179.6 E lands in the last column")


if __name__ == "__main__":
    unittest.main()
