#!/usr/bin/env python3
"""Tests for convert_glottolog.py on a few made-up rows."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import convert_glottolog as cg  # noqa: E402

CSV = """ID,Name,Macroarea,Latitude,Longitude,Glottocode,ISO639P3code,Level,Countries,Family_ID,Language_ID,Closest_ISO369P3code,First_Year_Of_Documentation,Last_Year_Of_Documentation,Is_Isolate
indo1319,Indo-European,Eurasia,,,indo1319,,family,,,,,,,
book1242,Bookkeeping,,,,book1242,,family,,,,,,,
stan1288,Spanish,Eurasia,40.4414,-1.117,stan1288,spa,language,ES,indo1319,,,,,false
basq1248,Basque,Eurasia,43.278687,-1.31636,basq1248,eus,language,ES;FR,,,,,,true
xxxx1234,Ledger,,10,10,xxxx1234,,language,,book1242,,,,,false
nolo1234,Nowhere,Africa,,,nolo1234,,language,,,,,,,false
dial1234,Some dialect,Eurasia,1,1,dial1234,,dialect,,indo1319,,,,,false
"""


class Convert(unittest.TestCase):
    def test_rows(self):
        lines = cg.convert(CSV).splitlines()
        self.assertEqual(lines[0], cg.MAGIC)
        self.assertEqual(lines[1:], [
            "basq1248\tBasque\t\t43.2787\t-1.3164\teus\tEurasia",
            "stan1288\tSpanish\tIndo-European\t40.4414\t-1.1170\tspa\tEurasia",
        ])

    def test_refuses_tabs(self):
        with self.assertRaises(ValueError):
            cg.convert(CSV.replace("Basque", "Bas\tque"))


if __name__ == "__main__":
    unittest.main()
