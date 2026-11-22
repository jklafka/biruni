#!/usr/bin/env python3
"""Build assets/data/history_languages.tsv: the language and language family
of the historical borders' polities, from the Seshat Global History
Databank's Polaris 2026 release (`seshat-polaris-2026`, CC BY-NC-SA 4.0),
joined to Cliopatria (`cliopatria-v021`, CC BY 4.0) by Cliopatria's
`SeshatID` column. The file stays separate from the borders because of its
share-alike license.

Seshat codes a polity's languages (`polity_language`) and linguistic
families (`polity_linguistic_family`), without dates, for about 450 of its
polities. Cliopatria names one or more Seshat IDs for about 60% of its
polity records (none after 1914 or so). A record gets the families and
languages of all its Seshat IDs, in the order Seshat lists them; its color
in the app is the first family. Family names are tidied (FAMILY_NAMES:
subgroups and spelling variants to the top-level family, as Seshat itself
uses for most polities) and non-values ("U", "NO_VALUE_ON_WIKI", ...)
dropped.

Where Seshat has nothing for a record (most polities after 1900, and many
before), Wikidata fills in (CC0; the committed snapshot
`data-src/wikidata_polity_languages.tsv`, see `fetch_wikidata_languages.py`):
the official languages (P37) of the record's Wikidata item, or, when it has
none, its languages used (P2936) if there are at most MAX_USED. Each
language's family is its Glottolog family (`glottolog-v54-languages`, by the
language's Glottolog code), named as Seshat names it (GLOTTOLOG_NAMES); the
main family is the one most of the languages share. Each P line says which
source it came from.

Consecutive records of one polity with the same values are merged into one
year range.

Stdlib only (the xlsx is read as zipped XML); the output is deterministic.

    python3 scripts/convert_seshat_languages.py         # fetch the pinned files, then convert
    python3 scripts/convert_seshat_languages.py POLARIS.xlsx CLIOPATRIA.zip GLOTTOLOG_LANGUAGES.csv

Format (UTF-8, tab-separated; read by src/history_lang.rs, checked by
scripts/validate_data.py):

    MAPSHISTLANG2
    F  <family>                         one line per family, most polity records first
    P  <Cliopatria name>  <from>  <to>  <family indexes, comma-separated>  <languages, "; "-separated>  <S (Seshat) or W (Wikidata)>

Years are Cliopatria's (calendar years, negative BCE, inclusive). A P line
applies to every record of that polity whose years lie within from..to.
"""
import csv
import io
import json
import os
import re
import sys
import zipfile
import xml.etree.ElementTree as ET

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

MAGIC = "MAPSHISTLANG2"
SOURCES = ("seshat-polaris-2026", "cliopatria-v021", "glottolog-v54-languages")
WIKIDATA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "data-src", "wikidata_polity_languages.tsv")
# Languages used (P2936) count only when a polity has at most this many.
MAX_USED = 3
# Languages shown per polity (the HUD line stays short).
MAX_LANGUAGES = 4
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "data", "history_languages.tsv")
# Seshat's values that are not families or languages.
NON_VALUES = {"", "U", "SU", "NO_VALUE_ON_WIKI", "suspected unknown", "none", "unknown"}
# Seshat family value -> the family shown.
FAMILY_NAMES = {
    "Indo-Iranian": "Indo-European",
    "Indo-Aryan": "Indo-European",
    "Iranian": "Indo-European",
    "Germanic": "Indo-European",
    "Celtic": "Indo-European",
    "Semitic": "Afro-Asiatic",
    "West Semetic": "Afro-Asiatic",
    "Hamito-Semitic": "Afro-Asiatic",
    "Chadic": "Afro-Asiatic",
    "Yoruboid": "Niger-Congo",
    "Kwa": "Niger-Congo",
    "Edoid": "Niger-Congo",
    "Proto-Bene-Kwa": "Niger-Congo",
    "Mande": "Niger-Congo",
    "Austro-Asiatic": "Austroasiatic",
    "Mon-Khmer": "Austroasiatic",
    "Oceanic-Austronesian": "Austronesian",
    "Malayo-Polynesian": "Austronesian",
    "Oghuz": "Turkic",
    "Proto-Mongolic": "Mongolic",
    "Manchu-Tungusic": "Tungusic",
    "Proto-Otomanguean": "Otomanguean",
    "Southern Dravidian": "Dravidian",
    "Iroquois": "Iroquoian",
    "isolate": "Isolate",
    "isolate language": "Isolate",
    "language isolate": "Isolate",
}

# Glottolog family name -> the name Seshat uses (FAMILY_NAMES above).
GLOTTOLOG_NAMES = {
    "Atlantic-Congo": "Niger-Congo",
    "Mande": "Niger-Congo",
    "Mongolic-Khitan": "Mongolic",
}
# Glottolog pseudo-families: not a genealogical family.
PSEUDO_FAMILIES = {
    "Artificial Language", "Bookkeeping", "Mixed Language", "Pidgin", "Sign Language",
    "Speech Register", "Unattested", "Unclassifiable",
}

NS = {"m": "http://schemas.openxmlformats.org/spreadsheetml/2006/main"}
REL = "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id"


def clean(text):
    if "\t" in text or "\n" in text:
        raise ValueError(f"tab or newline in {text!r}")
    return text


def xlsx_rows(path, sheet):
    """The rows of one sheet as lists of strings (empty cells "")."""
    with zipfile.ZipFile(path) as z:
        shared = []
        if "xl/sharedStrings.xml" in z.namelist():
            for si in ET.fromstring(z.read("xl/sharedStrings.xml")).findall("m:si", NS):
                shared.append("".join(t.text or "" for t in si.iter(f"{{{NS['m']}}}t")))
        book = ET.fromstring(z.read("xl/workbook.xml"))
        rid = next(s.get(REL) for s in book.find("m:sheets", NS) if s.get("name") == sheet)
        rels = ET.fromstring(z.read("xl/_rels/workbook.xml.rels"))
        target = next(r.get("Target") for r in rels if r.get("Id") == rid)
        target = target.lstrip("/")
        if not target.startswith("xl/"):
            target = "xl/" + target
        root = ET.fromstring(z.read(target))
    out = []
    for row in root.iter(f"{{{NS['m']}}}row"):
        cells = {}
        for c in row.findall("m:c", NS):
            col = re.match(r"[A-Z]+", c.get("r")).group(0)
            n = 0
            for ch in col:
                n = n * 26 + ord(ch) - 64
            v = c.find("m:v", NS)
            if c.get("t") == "s" and v is not None:
                text = shared[int(v.text)]
            elif c.get("t") == "inlineStr":
                text = "".join(t.text or "" for t in c.iter(f"{{{NS['m']}}}t"))
            else:
                text = v.text if v is not None and v.text is not None else ""
            cells[n - 1] = text
        out.append([cells.get(i, "") for i in range(max(cells) + 1)] if cells else [])
    return out


def seshat_values(general_rows):
    """{polity id: ([families], [languages])} from the General sheet's rows
    (a header row, then rows with polity_id, variable_name, value_from)."""
    header = general_rows[0]
    pid, var, val = (header.index(k) for k in ("polity_id", "variable_name", "value_from"))
    out = {}
    for r in general_rows[1:]:
        if len(r) <= max(pid, var, val):
            continue
        value = r[val].strip()
        if value in NON_VALUES:
            continue
        fams, langs = out.setdefault(r[pid], ([], []))
        if r[var] == "polity_linguistic_family":
            fam = FAMILY_NAMES.get(value, value)
            if fam not in fams:
                fams.append(clean(fam))
        elif r[var] == "polity_language" and value not in langs:
            langs.append(clean(value))
    return out


def seshat_ids(text):
    return [s.strip() for s in re.split(r"[;,]", text or "") if s.strip()]


def glottolog_families(rows):
    """{glottocode: family name as shown} from Glottolog's languages.csv
    rows; isolates are "Isolate", pseudo-families are left out."""
    names = {r["ID"]: r["Name"] for r in rows}
    out = {}
    for r in rows:
        if r["Family_ID"]:
            fam = names.get(r["Family_ID"], "")
        elif r["Level"] == "family":
            fam = r["Name"]
        else:
            fam = "Isolate"
        if fam and fam not in PSEUDO_FAMILIES:
            out[r["ID"]] = GLOTTOLOG_NAMES.get(fam, fam)
    return out


def wikidata_values(entries, families):
    """(families, languages) of one Wikidata item from its snapshot rows
    [(prop, language item, glottocode, label)]."""
    official = [e for e in entries if e[0] == "P37"]
    used = [e for e in entries if e[0] == "P2936"]
    if not official and len({e[1] for e in used}) > MAX_USED:
        used = []
    # One per language item (a language can have several Glottolog codes:
    # the first that has a family counts).
    by_item = {}
    for e in sorted(official or used, key=lambda e: (e[2] not in families, e[2])):
        by_item.setdefault(e[1], e)
    chosen = sorted(by_item.values(), key=lambda e: int(e[1][1:]) if e[1][1:].isdigit() else 0)
    langs = [clean(e[3]) for e in chosen if e[3]][:MAX_LANGUAGES]
    count, first = {}, {}
    for i, e in enumerate(chosen):
        fam = families.get(e[2])
        if fam:
            count[fam] = count.get(fam, 0) + 1
            first.setdefault(fam, i)
    fams = sorted(count, key=lambda f: (-count[f], first[f]))
    return fams, langs


def convert(general_rows, features, wikidata=None, glotto=None):
    """TSV text from Seshat's General sheet rows, Cliopatria's features, the
    Wikidata snapshot ({item: rows}) and Glottolog families ({glottocode:
    family})."""
    values = seshat_values(general_rows)
    wikidata, glotto = wikidata or {}, glotto or {}
    per_record = []
    for f in features:
        p = f["properties"]
        if p.get("Type") != "POLITY":
            continue
        fams, langs = [], []
        for sid in seshat_ids(p.get("SeshatID")):
            f_, l_ = values.get(sid, ([], []))
            fams += [x for x in f_ if x not in fams]
            langs += [x for x in l_ if x not in langs]
        source = "S"
        if not (fams or langs):
            fams, langs = wikidata_values(wikidata.get((p.get("Wikidata") or "").strip(), []), glotto)
            source = "W"
        if fams or langs:
            per_record.append((clean(p["Name"]), int(p["FromYear"]), int(p["ToYear"]), tuple(fams), tuple(langs), source))
    per_record = sorted(set(per_record), key=lambda r: (r[0], r[1], r[2], r[3], r[4], r[5]))
    # Families by number of records, then name.
    count = {}
    for r in per_record:
        for fam in r[3]:
            count[fam] = count.get(fam, 0) + 1
    families = sorted(count, key=lambda f: (-count[f], f))
    index = {f: i for i, f in enumerate(families)}
    merged = []
    for name, start, end, fams, langs, source in per_record:
        last = merged[-1] if merged else None
        if last and last[0] == name and last[3:] == [fams, langs, source] and start <= last[2] + 1:
            last[2] = max(last[2], end)
        else:
            merged.append([name, start, end, fams, langs, source])
    lines = [MAGIC] + [f"F\t{f}" for f in families]
    for name, start, end, fams, langs, source in merged:
        lines.append("\t".join(
            ["P", name, str(start), str(end), ",".join(str(index[f]) for f in fams), "; ".join(langs), source]
        ))
    return "\n".join(lines) + "\n"


def load_features(path):
    with zipfile.ZipFile(path) as z:
        member = next(n for n in sorted(z.namelist()) if n.endswith(".geojson"))
        with z.open(member) as f:
            return json.load(io.TextIOWrapper(f, encoding="utf-8"))["features"]


def main(argv):
    if argv:
        polaris, clio, glotto = argv[0], argv[1], argv[2]
    else:
        import data_sources

        polaris, clio, glotto = (data_sources.fetch(s) for s in SOURCES)
    import fetch_wikidata_languages

    with open(WIKIDATA, encoding="utf-8") as f:
        wikidata = fetch_wikidata_languages.parse(f.read())
    with open(glotto, encoding="utf-8", newline="") as f:
        families = glottolog_families(list(csv.DictReader(f)))
    text = convert(xlsx_rows(polaris, "General"), load_features(clio), wikidata, families)
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    lines = text.splitlines()
    fams = sum(1 for line in lines if line.startswith("F\t"))
    print(f"wrote {OUT}: {len(lines) - 1 - fams} polity ranges, {fams} families", file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv[1:])
