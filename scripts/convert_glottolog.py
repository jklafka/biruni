#!/usr/bin/env python3
"""Build assets/data/languages.tsv, the languages layer, from Glottolog 5.4
(glottolog-cldf, CC BY 4.0), pinned in data-sources.lock.json as
`glottolog-v54-languages`.

Keeps the languoids of level "language" that have a location, except the
"Bookkeeping" pseudo-family (entries Glottolog keeps for bookkeeping, not
languages). Each language gets its top-level family's name, empty for
isolates. Rows are sorted by glottocode. Stdlib only; deterministic.

    python3 scripts/convert_glottolog.py              # fetch the pinned file, then convert
    python3 scripts/convert_glottolog.py languages.csv

Format (UTF-8, tab-separated, a header line, then one row per language;
read by src/languages.rs, checked by scripts/validate_data.py):

    MAPSLANGS1
    glottocode  name  family  lat  lon  iso639-3  macroarea

Coordinates have 4 decimals (about 10 m). Tabs and newlines never occur in
the source fields; the converter refuses them anyway.
"""
import csv
import io
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

SOURCE = "glottolog-v54-languages"
MAGIC = "MAPSLANGS1"
BOOKKEEPING = "book1242"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "data", "languages.tsv")


def convert(text):
    rows = list(csv.DictReader(io.StringIO(text)))
    names = {r["ID"]: r["Name"] for r in rows}
    out = []
    for r in rows:
        if r["Level"] != "language" or not r["Latitude"] or not r["Longitude"]:
            continue
        if r["Family_ID"] == BOOKKEEPING:
            continue
        lat, lon = float(r["Latitude"]), float(r["Longitude"])
        if not (-90 <= lat <= 90 and -180 <= lon <= 180):
            continue
        family = names.get(r["Family_ID"], "") if r["Family_ID"] else ""
        fields = [r["ID"], r["Name"], family, f"{lat:.4f}", f"{lon:.4f}", r["ISO639P3code"], r["Macroarea"]]
        if any("\t" in f or "\n" in f for f in fields):
            raise ValueError(f"tab or newline in {fields}")
        out.append(fields)
    out.sort(key=lambda f: f[0])
    return MAGIC + "\n" + "".join("\t".join(f) + "\n" for f in out)


def main(argv):
    if argv:
        path = argv[0]
    else:
        import data_sources

        path = data_sources.fetch(SOURCE)
    with open(path, encoding="utf-8", newline="") as f:
        text = convert(f.read())
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"wrote {OUT}: {text.count(chr(10)) - 1} languages", file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv[1:])
