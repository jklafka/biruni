#!/usr/bin/env python3
"""Tests for fetch_wikidata_languages.py without the network."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import fetch_wikidata_languages as fw  # noqa: E402

ROOT = Path(__file__).parent.parent


def uri(q):
    return {"type": "uri", "value": f"http://www.wikidata.org/entity/{q}"}


class Fetch(unittest.TestCase):
    def test_items(self):
        features = [
            {"properties": {"Type": "POLITY", "Wikidata": "Q100"}},
            {"properties": {"Type": "POLITY", "Wikidata": "Q9"}},
            {"properties": {"Type": "POLITY", "Wikidata": "Q9"}},
            {"properties": {"Type": "POLITY", "Wikidata": ""}},
            {"properties": {"Type": "POLITY", "Wikidata": "not-an-item"}},
            {"properties": {"Type": "RELATION", "Wikidata": "Q5"}},
        ]
        self.assertEqual(fw.polity_items(features), ["Q9", "Q100"])

    def test_rows_round_trip(self):
        bindings = [
            {"polity": uri("Q16"), "prop": {"value": "P37"}, "lang": uri("Q150"),
             "glotto": {"value": "stan1290"}, "label": {"value": "French"}},
            {"polity": uri("Q16"), "prop": {"value": "P37"}, "lang": uri("Q150"),
             "glotto": {"value": "stan1290"}, "label": {"value": "French"}},
            {"polity": uri("Q2"), "prop": {"value": "P2936"}, "lang": uri("Q7"),
             "label": {"value": "Tab\there"}},
            # No code of its own: the lowest of its parents'.
            {"polity": uri("Q148"), "prop": {"value": "P37"}, "lang": uri("Q727694"),
             "parentGlotto": {"value": "mand1415"}, "label": {"value": "Putonghua"}},
            {"polity": uri("Q148"), "prop": {"value": "P37"}, "lang": uri("Q727694"),
             "parentGlotto": {"value": "beij1234"}, "label": {"value": "Putonghua"}},
            # Two steps up only when no parent has one.
            {"polity": uri("Q148"), "prop": {"value": "P37"}, "lang": uri("Q24841726"),
             "grandGlotto": {"value": "beij1235"}, "label": {"value": "Putonghua"}},
            {"polity": uri("Q148"), "prop": {"value": "P37"}, "lang": uri("Q727694"),
             "grandGlotto": {"value": "aaaa1111"}, "label": {"value": "Putonghua"}},
            # Its own code wins over a parent's.
            {"polity": uri("Q16"), "prop": {"value": "P37"}, "lang": uri("Q150"),
             "glotto": {"value": "stan1290"}, "parentGlotto": {"value": "oilf1242"},
             "label": {"value": "French"}},
        ]
        rows = fw.rows_from_bindings(bindings)
        self.assertEqual(len(rows), 4, "duplicates collapse")
        text = fw.format_rows(rows, "2026-11-22")
        self.assertEqual(text.splitlines(), [
            "MAPSWDLANG1\t2026-11-22",
            "Q2\tP2936\tQ7\t\tTab here",
            "Q16\tP37\tQ150\tstan1290\tFrench",
            "Q148\tP37\tQ727694\tbeij1234\tPutonghua",
            "Q148\tP37\tQ24841726\tbeij1235\tPutonghua",
        ])
        self.assertEqual(fw.parse(text)["Q16"], [("P37", "Q150", "stan1290", "French")])
        with self.assertRaises(ValueError):
            fw.parse("X\n")
        with self.assertRaises(ValueError):
            fw.parse("MAPSWDLANG1\t2026-11-22\nQ1\tP37\n")

    def test_the_snapshot_parses(self):
        text = (ROOT / "data-src" / "wikidata_polity_languages.tsv").read_text(encoding="utf-8")
        data = fw.parse(text)
        self.assertGreater(len(data), 500)
        self.assertEqual(text, fw.format_rows({(k, *v) for k, vs in data.items() for v in vs},
                                              text.split("\t", 2)[1].split("\n")[0]), "sorted, no duplicates")


if __name__ == "__main__":
    unittest.main()
