#!/usr/bin/env python3
"""Tests for convert_dplace.py on a few made-up rows."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import convert_dplace as cd  # noqa: E402


def soc(id_, name, lat, lon, contribution="dplace-dataset-ea"):
    return {"ID": id_, "Name": name, "Latitude": lat, "Longitude": lon, "region": "R",
            "main_focal_year": "1900", "Contribution_ID": contribution}


SOCIETIES = [soc("Aa2", "Two", "1", "2"), soc("Aa1", "One", "-20", "21"),
             soc("B1", "Binford", "1", "1", "dplace-dataset-binford"), soc("Ab1", "Nowhere", "", "")]
CODES = [{"ID": "EA042-1", "Var_ID": "EA042", "Name": "Gathering"},
         {"ID": "EA042-2", "Var_ID": "EA042", "Name": "Fishing"},
         {"ID": "EA042-NA", "Var_ID": "EA042", "Name": "Missing"},
         {"ID": "EA043-1", "Var_ID": "EA043", "Name": "Patrilineal"},
         {"ID": "EA999-1", "Var_ID": "EA999", "Name": "Other"}]
DATA = [{"Soc_ID": "Aa1", "Var_ID": "EA042", "Code_ID": "EA042-1"},
        {"Soc_ID": "Aa2", "Var_ID": "EA042", "Code_ID": "EA042-NA"},
        {"Soc_ID": "Aa2", "Var_ID": "EA043", "Code_ID": "EA043-1"},
        {"Soc_ID": "Aa1", "Var_ID": "EA999", "Code_ID": "EA999-1"}]


class Convert(unittest.TestCase):
    def test_output(self):
        lines = cd.convert(SOCIETIES, CODES, DATA).splitlines()
        self.assertEqual(lines[0], cd.MAGIC)
        v = [l for l in lines if l.startswith("V\t")]
        self.assertEqual(len(v), len(cd.VARIABLES))
        self.assertEqual(v[0], "V\tEA042\tSubsistence: dominant activity\t1=Gathering|2=Fishing")
        s = [l.split("\t") for l in lines if l.startswith("S\t")]
        self.assertEqual([r[1] for r in s], ["Aa1", "Aa2"], "EA only, located, sorted")
        self.assertEqual(s[0][3:5], ["-20.00", "21.00"])
        self.assertEqual(s[0][7:9], ["1", ""])
        self.assertEqual(s[1][7:9], ["", "1"], "NA is empty")

    def test_refuses_tabs(self):
        with self.assertRaises(ValueError):
            cd.convert([soc("Aa1", "Tab\there", "1", "1")], CODES, DATA)


if __name__ == "__main__":
    unittest.main()
