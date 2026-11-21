#!/usr/bin/env python3
"""Tests for convert_merdith.py on a tiny made-up GPML and rotation file."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import convert_merdith as cm  # noqa: E402
import hist_format as hf  # noqa: E402


def square(lat, lon, size):
    pts = [(lat, lon), (lat, lon + size), (lat + size, lon + size), (lat + size, lon), (lat, lon)]
    return " ".join(f"{a} {b}" for a, b in pts)


def feature(name, pid, begin, end, poslist):
    return f"""<gml:featureMember><gpml:UnclassifiedFeature>
<gpml:unclassifiedGeometry><gpml:ConstantValue><gpml:value><gml:Polygon><gml:exterior><gml:LinearRing>
<gml:posList gml:dimension="2">{poslist}</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon>
</gpml:value></gpml:ConstantValue></gpml:unclassifiedGeometry>
<gpml:reconstructionPlateId><gpml:ConstantValue><gpml:value>{pid}</gpml:value></gpml:ConstantValue></gpml:reconstructionPlateId>
<gml:validTime><gml:TimePeriod><gml:begin><gml:TimeInstant><gml:timePosition>{begin}</gml:timePosition></gml:TimeInstant></gml:begin>
<gml:end><gml:TimeInstant><gml:timePosition>{end}</gml:timePosition></gml:TimeInstant></gml:end></gml:TimePeriod></gml:validTime>
<gml:name>{name}</gml:name></gpml:UnclassifiedFeature></gml:featureMember>"""


GPML = f"""<?xml version="1.0"?><gpml:FeatureCollection xmlns:gml="http://www.opengis.net/gml" xmlns:gpml="http://www.gplates.org/gplates">
{feature("Big", 201, "1800", "0", square(-10, -60, 10))}
{feature("Young", 701, "100", "http://gplates.org/times/distantFuture", square(0, 20, 5))}
{feature("Old", 301, "http://gplates.org/times/distantPast", "600", square(40, 10, 5))}
{feature("Speck", 801, "50", "0", square(0, 0, 0.01))}
</gpml:FeatureCollection>""".encode()

ROT = """001  0.0   90.0    0.0    0.0  000 ! fixed
201  0.0   90.0    0.0    0.0  001 ! South America
201 100.0  50.0  -30.0   20.5  001 !
999  0.0   90.0    0.0    0.0  000 ! comment
bad line
"""


class Convert(unittest.TestCase):
    def test_coastlines(self):
        scale, digits, names, geoms, records = hf.decode(cm.coastlines(GPML))
        self.assertEqual((scale, digits), (hf.SCALE_MA, cm.DIGITS))
        got = sorted((names[r[0]], r[1], r[2], r[5]) for r in records)
        # Ages are stored as -(tenths of Ma); 1800 Ma is clamped to 1000.
        self.assertEqual(got, [("Big", -10000, 0, 201), ("Old", -10000, -6000, 301), ("Young", -1000, 0, 701)])
        big = next(r for r in records if names[r[0]] == "Big")
        lon, lat = geoms[big[3]][0][0]
        self.assertEqual((lon, lat), (-60.0, -10.0), "GPML lists latitude first")

    def test_rotations(self):
        lines = cm.rotations(ROT).splitlines()
        self.assertEqual(lines, ["MAPSROT1", "1\t0.0\t90.0\t0.0\t0.0\t0", "201\t0.0\t90.0\t0.0\t0.0\t1",
                                 "201\t100.0\t50.0\t-30.0\t20.5\t1"])


if __name__ == "__main__":
    unittest.main()
