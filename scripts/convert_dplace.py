#!/usr/bin/env python3
"""Build assets/data/societies.tsv, the societies layer, from D-PLACE 3.3.0
(dplace-cldf, CC BY-NC 4.0): the Ethnographic Atlas (Murdock et al.)
societies, where they lived, and six of its coded variables. Sources are
pinned in data-sources.lock.json as `dplace-v330-*`.

Stdlib only; the output is deterministic.

    python3 scripts/convert_dplace.py                  # fetch the pinned files, then convert
    python3 scripts/convert_dplace.py DIR              # convert local copies in DIR

Format (UTF-8, tab-separated; read by src/ethnography.rs, checked by
scripts/validate_data.py):

    MAPSSOCIETIES1
    V  <variable id>  <title>  <code>=<label>|<code>=<label>|...
    ...                         one V line per variable, in column order
    S  <id>  <name>  <lat>  <lon>  <region>  <focal year>  <code per variable>...

A society's code is empty when the Atlas has no value for it (codes the
Atlas marks as missing, "NA", count as empty). Coordinates have 2 decimals.
Rows are sorted by society id.
"""
import csv
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

MAGIC = "MAPSSOCIETIES1"
CONTRIBUTION = "dplace-dataset-ea"
# Variable id and the short title the legend shows.
VARIABLES = [
    ("EA042", "Subsistence: dominant activity"),
    ("EA043", "Descent"),
    ("EA012", "Marital residence"),
    ("EA030", "Settlement pattern"),
    ("EA033", "Political levels above the community"),
    ("EA066", "Class differentiation"),
]
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "data", "societies.tsv")


def clean(text):
    if "\t" in text or "\n" in text or "|" in text:
        raise ValueError(f"tab, newline or bar in {text!r}")
    return text


def convert(societies, codes, data):
    """TSV text from the rows of societies.csv, codes.csv and data.csv."""
    wanted = [v for v, _ in VARIABLES]
    labels = {v: [] for v in wanted}
    for c in codes:
        if c["Var_ID"] in labels and not c["ID"].endswith("-NA"):
            labels[c["Var_ID"]].append((int(c["ID"].rsplit("-", 1)[1]), c["Name"]))
    values = {}
    for d in data:
        if d["Var_ID"] in labels and d["Code_ID"] and not d["Code_ID"].endswith("-NA"):
            values[(d["Soc_ID"], d["Var_ID"])] = d["Code_ID"].rsplit("-", 1)[1]
    lines = [MAGIC]
    for v, title in VARIABLES:
        pairs = "|".join(f"{n}={clean(name)}" for n, name in sorted(labels[v]))
        lines.append("\t".join(["V", v, clean(title), pairs]))
    rows = []
    for s in societies:
        if s["Contribution_ID"] != CONTRIBUTION or not s["Latitude"] or not s["Longitude"]:
            continue
        lat, lon = float(s["Latitude"]), float(s["Longitude"])
        if not (-90 <= lat <= 90 and -180 <= lon <= 180):
            continue
        row = ["S", s["ID"], clean(s["Name"]), f"{lat:.2f}", f"{lon:.2f}", clean(s["region"]), s["main_focal_year"]]
        row += [values.get((s["ID"], v), "") for v in wanted]
        rows.append(row)
    rows.sort(key=lambda r: r[1])
    lines += ["\t".join(r) for r in rows]
    return "\n".join(lines) + "\n"


def read(path):
    with open(path, encoding="utf-8", newline="") as f:
        return list(csv.DictReader(f))


def main(argv):
    if argv:
        paths = {k: os.path.join(argv[0], f"{k}.csv") for k in ("societies", "codes", "data")}
    else:
        import data_sources

        paths = {k: data_sources.fetch(f"dplace-v330-{k}") for k in ("societies", "codes", "data")}
    text = convert(read(paths["societies"]), read(paths["codes"]), read(paths["data"]))
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"wrote {OUT}: {text.count(chr(10)) - 1 - len(VARIABLES)} societies", file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv[1:])
