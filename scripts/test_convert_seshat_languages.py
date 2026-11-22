#!/usr/bin/env python3
"""Tests for convert_seshat_languages.py on made-up rows and a tiny xlsx."""
import os
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import convert_seshat_languages as cs  # noqa: E402

GENERAL = [
    ["seshat_region", "polity_number", "polity_id", "section", "variable_name", "value_from"],
    ["Italy", 1, "it_roman_rep", "general", "polity_language", "Latin"],
    ["Italy", 1, "it_roman_rep", "general", "polity_linguistic_family", "Indo-European"],
    ["Italy", 2, "it_roman_emp", "general", "polity_language", "Latin"],
    ["Italy", 2, "it_roman_emp", "general", "polity_language", "Greek"],
    ["Italy", 2, "it_roman_emp", "general", "polity_linguistic_family", "Germanic"],
    ["Egypt", 3, "eg_ptol", "general", "polity_linguistic_family", "Semitic"],
    ["Egypt", 3, "eg_ptol", "general", "polity_linguistic_family", "Indo-European"],
    ["Egypt", 3, "eg_ptol", "general", "polity_language", "U"],
    ["Nowhere", 4, "xx_none", "general", "polity_linguistic_family", "NO_VALUE_ON_WIKI"],
    ["Short"],
]


def feature(name, start, end, sid, kind="POLITY", wd=""):
    return {"properties": {"Name": name, "FromYear": start, "ToYear": end, "SeshatID": sid, "Type": kind,
                           "Wikidata": wd}}


GLOTTOLOG = [
    {"ID": "indo1319", "Name": "Indo-European", "Level": "family", "Family_ID": ""},
    {"ID": "atla1278", "Name": "Atlantic-Congo", "Level": "family", "Family_ID": ""},
    {"ID": "pidg1258", "Name": "Pidgin", "Level": "family", "Family_ID": ""},
    {"ID": "stan1290", "Name": "French", "Level": "language", "Family_ID": "indo1319"},
    {"ID": "stan1293", "Name": "English", "Level": "language", "Family_ID": "indo1319"},
    {"ID": "yoru1245", "Name": "Yoruba", "Level": "language", "Family_ID": "atla1278"},
    {"ID": "nige1257", "Name": "Naija", "Level": "language", "Family_ID": "pidg1258"},
    {"ID": "basq1248", "Name": "Basque", "Level": "language", "Family_ID": ""},
]

WIKIDATA = {
    # Official languages win over languages used.
    "Q1": [("P2936", "Q9", "yoru1245", "Yoruba"), ("P37", "Q150", "stan1290", "French"),
           ("P37", "Q1860", "stan1293", "English")],
    # No official language: up to MAX_USED languages used count.
    "Q2": [("P2936", "Q34", "yoru1245", "Yoruba"), ("P2936", "Q35", "basq1248", "Basque"),
           ("P2936", "Q36", "yoru1245", "Yoruba dialect")],
    # Too many languages used: nothing.
    "Q3": [("P2936", f"Q{i}", "stan1290", f"L{i}") for i in range(10, 15)],
    # A language with two codes: the one Glottolog knows counts.
    "Q5": [("P37", "Q150", "dani1284", "French"), ("P37", "Q150", "stan1290", "French")],
    # A pidgin has no family; the label still counts.
    "Q4": [("P37", "Q5", "nige1257", "Naija")],
}

FEATURES = [
    feature("Roman Republic", -500, -301, "it_roman_rep"),
    feature("Roman Republic", -300, -32, "it_roman_rep"),
    feature("Roman Empire", -31, 100, "it_roman_emp"),
    feature("Roman Empire", 150, 200, "it_roman_emp"),
    feature("Ptolemaic Kingdom", -300, -30, "eg_ptol; it_roman_rep"),
    feature("Nowhere", 1, 2, "xx_none"),
    feature("Unlinked", 1, 2, ""),
    feature("Union", 1, 2, "it_roman_rep", kind="RELATION"),
]


class Convert(unittest.TestCase):
    def test_output(self):
        lines = cs.convert(GENERAL, FEATURES).splitlines()
        self.assertEqual(lines[0], cs.MAGIC)
        self.assertEqual(lines[1:3], ["F\tIndo-European", "F\tAfro-Asiatic"])
        self.assertEqual(lines[3:], [
            # Families from both IDs, first family first; "U" dropped.
            "P\tPtolemaic Kingdom\t-300\t-30\t1,0\tLatin\tS",
            # Germanic is Indo-European; the gap from 101 to 149 splits the range.
            "P\tRoman Empire\t-31\t100\t0\tLatin; Greek\tS",
            "P\tRoman Empire\t150\t200\t0\tLatin; Greek\tS",
            # Adjacent records with the same values merge.
            "P\tRoman Republic\t-500\t-32\t0\tLatin\tS",
        ])

    def test_wikidata_fills_gaps(self):
        families = cs.glottolog_families(GLOTTOLOG)
        self.assertEqual(families["yoru1245"], "Niger-Congo", "named as Seshat names it")
        self.assertEqual(families["basq1248"], "Isolate")
        self.assertNotIn("nige1257", families, "pseudo-families are not families")
        features = FEATURES + [
            feature("Third Republic", 1871, 1939, "", wd="Q1"),
            feature("Oyo", 1600, 1700, "", wd="Q2"),
            feature("Babel", 1, 2, "", wd="Q3"),
            feature("Lagos", 1, 2, "", wd="Q4"),
            feature("Two codes", 1, 2, "", wd="Q5"),
            # Seshat wins where it has data.
            feature("Roman Republic", -31, -20, "it_roman_rep", wd="Q1"),
        ]
        lines = cs.convert(GENERAL, features, WIKIDATA, families).splitlines()
        fams = [line[2:] for line in lines if line.startswith("F\t")]
        self.assertEqual(fams, ["Indo-European", "Afro-Asiatic", "Isolate", "Niger-Congo"], "by records, then name")
        nc, iso = fams.index("Niger-Congo"), fams.index("Isolate")
        rows = [line for line in lines if line.startswith("P\t")]
        self.assertIn("P\tThird Republic\t1871\t1939\t0\tFrench; English\tW", rows)
        # Two of three languages used are Niger-Congo: that is the main family.
        self.assertIn(f"P\tOyo\t1600\t1700\t{nc},{iso}\tYoruba; Basque; Yoruba dialect\tW", rows)
        self.assertIn("P\tLagos\t1\t2\t\tNaija\tW", rows)
        self.assertIn("P\tTwo codes\t1\t2\t0\tFrench\tW", rows)
        self.assertFalse(any("Babel" in r for r in rows))
        self.assertIn("P\tRoman Republic\t-500\t-20\t0\tLatin\tS", rows)

    def test_refuses_tabs(self):
        with self.assertRaises(ValueError):
            cs.convert(GENERAL, [feature("Bad\tname", 1, 2, "it_roman_rep")])

    def test_reads_xlsx(self):
        shared = ["polity_id", "variable_name", "value_from", "it_x", "polity_language", "Latin"]
        sst = "".join(f"<si><t>{s}</t></si>" for s in shared)
        ns = 'xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"'
        rns = 'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'
        sheet = (f'<worksheet {ns}><sheetData>'
                 '<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" t="s"><v>2</v></c></row>'
                 '<row r="2"><c r="A2" t="s"><v>3</v></c><c r="B2" t="s"><v>4</v></c><c r="D2"><v>7</v></c></row>'
                 '<row r="3"><c r="A3" t="inlineStr"><is><t>it_y</t></is></c><c r="C3" t="s"><v>5</v></c></row>'
                 '</sheetData></worksheet>')
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "t.xlsx")
            with zipfile.ZipFile(path, "w") as z:
                z.writestr("xl/sharedStrings.xml", f"<sst {ns}>{sst}</sst>")
                z.writestr("xl/workbook.xml", f'<workbook {ns} {rns}><sheets>'
                           '<sheet name="Other" sheetId="1" r:id="rId1"/><sheet name="General" sheetId="2" r:id="rId2"/>'
                           '</sheets></workbook>')
                z.writestr("xl/_rels/workbook.xml.rels",
                           '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
                           '<Relationship Id="rId1" Target="worksheets/sheet1.xml"/>'
                           '<Relationship Id="rId2" Target="/xl/worksheets/sheet2.xml"/></Relationships>')
                z.writestr("xl/worksheets/sheet1.xml", f"<worksheet {ns}><sheetData/></worksheet>")
                z.writestr("xl/worksheets/sheet2.xml", sheet)
            rows = cs.xlsx_rows(path, "General")
        self.assertEqual(rows, [
            ["polity_id", "variable_name", "value_from"],
            ["it_x", "polity_language", "", "7"],
            ["it_y", "", "Latin"],
        ])


if __name__ == "__main__":
    unittest.main()
