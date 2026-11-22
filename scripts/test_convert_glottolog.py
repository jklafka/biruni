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

VALUES = """ID,Language_ID,Parameter_ID,Value,Code_ID,Comment,Source,codeReference
basq1248-aes,basq1248,aes,2,aes-threatened,,,
stan1288-aes,stan1288,aes,1,aes-not_endangered,,,
stan1288-med,stan1288,med,0,med-long_grammar,,,
"""


def gb_codes():
    rows = []
    for fid, _ in cg.FEATURES:
        if fid == "GB024":
            rows += [{"Parameter_ID": fid, "Name": "1", "Description": "Num-N"},
                     {"Parameter_ID": fid, "Name": "2", "Description": "N-Num"},
                     {"Parameter_ID": fid, "Name": "3", "Description": "both."}]
        else:
            rows += [{"Parameter_ID": fid, "Name": "0", "Description": "absent"},
                     {"Parameter_ID": fid, "Name": "1", "Description": "present"}]
    return rows


GB_LANGUAGES = [
    {"ID": "stan1288", "Language_level_ID": "stan1288"},
    # A dialect of Basque coded in place of the language.
    {"ID": "gipu1234", "Language_level_ID": "basq1248"},
    {"ID": "bizk1234", "Language_level_ID": "basq1248"},
]
GB_VALUES = [
    {"Language_ID": "stan1288", "Parameter_ID": "GB020", "Value": "1"},
    {"Language_ID": "stan1288", "Parameter_ID": "GB024", "Value": "1"},
    {"Language_ID": "stan1288", "Parameter_ID": "GB030", "Value": "?"},
    {"Language_ID": "stan1288", "Parameter_ID": "GB999", "Value": "1"},
    {"Language_ID": "gipu1234", "Parameter_ID": "GB020", "Value": "0"},
    {"Language_ID": "bizk1234", "Parameter_ID": "GB020", "Value": "1"},
    {"Language_ID": "bizk1234", "Parameter_ID": "GB024", "Value": "3"},
]


def convert(**over):
    args = dict(languages_text=CSV, values_text=VALUES, gb_values=GB_VALUES,
                gb_codes=gb_codes(), gb_languages=GB_LANGUAGES)
    args.update(over)
    return cg.convert(**args)


class Convert(unittest.TestCase):
    def test_variables(self):
        lines = convert().splitlines()
        self.assertEqual(lines[0], cg.MAGIC)
        vs = [line.split("\t") for line in lines if line.startswith("V\t")]
        self.assertEqual(len(vs), 1 + len(cg.FEATURES))
        self.assertEqual(vs[0][1], "aes")
        self.assertEqual(vs[0][4].split("|")[-1], "6=Extinct")
        gb020 = vs[1 + [f for f, _ in cg.FEATURES].index("GB020")]
        self.assertEqual(gb020[4], "0=No|1=Yes")
        gb024 = vs[1 + [f for f, _ in cg.FEATURES].index("GB024")]
        self.assertEqual(gb024[4], "1=Num-N|2=N-Num|3=Both")

    def test_rows(self):
        rows = [line.split("\t") for line in convert().splitlines() if line.startswith("L\t")]
        self.assertEqual([r[1] for r in rows], ["basq1248", "stan1288"], "sorted; no ledger, dialect or unlocated")
        self.assertEqual(rows[0][2:8], ["Basque", "", "43.2787", "-1.3164", "eus", "Eurasia"])
        self.assertEqual(rows[1][3], "Indo-European")
        self.assertEqual(len(rows[0]), 8 + 1 + len(cg.FEATURES))
        ids = [f for f, _ in cg.FEATURES]
        col = lambda fid: 9 + ids.index(fid)  # noqa: E731
        self.assertEqual(rows[0][8], "2", "endangerment")
        self.assertEqual(rows[1][8], "1")
        # The first coded dialect (by Grambank id) stands in for Basque.
        self.assertEqual(rows[0][col("GB020")], "1")
        self.assertEqual(rows[0][col("GB024")], "3")
        self.assertEqual(rows[1][col("GB020")], "1")
        self.assertEqual(rows[1][col("GB030")], "", "? is no data")
        self.assertEqual(rows[0][col("GB057")], "")

    def test_the_language_beats_its_dialects(self):
        gb = GB_LANGUAGES + [{"ID": "basq1248", "Language_level_ID": "basq1248"}]
        values = GB_VALUES + [{"Language_ID": "basq1248", "Parameter_ID": "GB020", "Value": "0"}]
        rows = [line.split("\t") for line in convert(gb_values=values, gb_languages=gb).splitlines()
                if line.startswith("L\tbasq")]
        self.assertEqual(rows[0][9 + [f for f, _ in cg.FEATURES].index("GB020")], "0")

    def test_refuses_bad_input(self):
        with self.assertRaises(ValueError):
            convert(languages_text=CSV.replace("Basque", "Bas\tque"))
        with self.assertRaises(ValueError):
            convert(gb_values=GB_VALUES + [{"Language_ID": "stan1288", "Parameter_ID": "GB057", "Value": "7"}])
        with self.assertRaises(ValueError):
            convert(gb_codes=[c for c in gb_codes() if c["Parameter_ID"] != "GB415"])
        # A bar in a name is fine (it is not a label).
        self.assertIn("Bas|que", convert(languages_text=CSV.replace("Basque", "Bas|que")))


if __name__ == "__main__":
    unittest.main()
