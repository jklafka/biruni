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


def feature(name, start, end, sid, kind="POLITY"):
    return {"properties": {"Name": name, "FromYear": start, "ToYear": end, "SeshatID": sid, "Type": kind}}


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
            "P\tPtolemaic Kingdom\t-300\t-30\t1,0\tLatin",
            # Germanic is Indo-European; the gap from 101 to 149 splits the range.
            "P\tRoman Empire\t-31\t100\t0\tLatin; Greek",
            "P\tRoman Empire\t150\t200\t0\tLatin; Greek",
            # Adjacent records with the same values merge.
            "P\tRoman Republic\t-500\t-32\t0\tLatin",
        ])

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
