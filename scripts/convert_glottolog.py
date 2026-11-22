#!/usr/bin/env python3
"""Build assets/data/languages.tsv, the languages layer, from Glottolog 5.4
(glottolog-cldf, CC BY 4.0) and Grambank 1.0.3 (CC BY 4.0), pinned in
data-sources.lock.json as `glottolog-v54-languages`, `glottolog-v54-values`
and `grambank-v103-{values,codes,languages}`.

Keeps the Glottolog languoids of level "language" that have a location,
except the "Bookkeeping" pseudo-family (entries Glottolog keeps for
bookkeeping, not languages). Each language gets its top-level family's name
(empty for isolates) and one code per variable the layer can color by:

- `aes`, Glottolog's agglomerated endangerment status (1 not endangered ..
  6 extinct), from `values.csv`;
- the Grambank features in FEATURES, from Grambank's `values.csv`. Grambank
  codes some dialects instead of their language; a language takes its own
  coding if Grambank has one, else that of its first coded dialect (by
  Grambank ID). "?" (unknown) counts as no data.

Rows are sorted by glottocode. Stdlib only; deterministic.

    python3 scripts/convert_glottolog.py         # fetch the pinned files, then convert
    python3 scripts/convert_glottolog.py DIR     # convert local copies in DIR (see LOCAL)

Format (UTF-8, tab-separated; read by src/languages.rs, checked by
scripts/validate_data.py):

    MAPSLANGS2
    V  <variable id>  <title>  <question>  <code>=<label>|<code>=<label>|...
    ...                               one V line per variable, in column order
    L  <glottocode>  <name>  <family>  <lat>  <lon>  <iso639-3>  <macroarea>  <code per variable>...

A code is empty when the source has no value. Coordinates have 4 decimals
(about 10 m). Tabs and newlines never occur in the source fields, nor bars
in titles and labels; the converter refuses them anyway.
"""
import csv
import io
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

MAGIC = "MAPSLANGS2"
BOOKKEEPING = "book1242"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "data", "languages.tsv")
# Source name -> file name in a local directory.
LOCAL = {
    "glottolog-v54-languages": "languages.csv",
    "glottolog-v54-values": "values.csv",
    "grambank-v103-values": "grambank_values.csv",
    "grambank-v103-codes": "grambank_codes.csv",
    "grambank-v103-languages": "grambank_languages.csv",
}

ENDANGERMENT = (
    "aes",
    "Endangerment",
    "Glottolog's agglomerated endangerment status",
    [(1, "Not endangered"), (2, "Threatened"), (3, "Shifting"), (4, "Moribund"), (5, "Nearly extinct"), (6, "Extinct")],
)
# Grambank feature id, the legend's short title. The question is Grambank's.
FEATURES = [
    ("GB020", "Definite articles"),
    ("GB030", "\"He\" and \"she\" differ"),
    ("GB051", "Gender by sex"),
    ("GB057", "Numeral classifiers"),
    ("GB028", "Inclusive and exclusive \"we\""),
    ("GB070", "Case marking on nouns"),
    ("GB409", "Ergative case marking"),
    ("GB133", "Verb last (as in SOV)"),
    ("GB131", "Verb first (as in VSO)"),
    ("GB024", "Numeral and noun order"),
    ("GB074", "Prepositions"),
    ("GB075", "Postpositions"),
    ("GB323", "Evidentials (hearsay, inference)"),
    ("GB415", "Polite \"you\""),
    ("GB335", "Base-20 numerals"),
]
BINARY_LABELS = {"absent": "No", "present": "Yes"}


def clean(text):
    if "\t" in text or "\n" in text or "|" in text:
        raise ValueError(f"tab, newline or bar in {text!r}")
    return text


def label(description):
    """A code's label: Yes/No for binary features, else the description
    without a final period, capitalized."""
    if description in BINARY_LABELS:
        return BINARY_LABELS[description]
    d = description.rstrip(".")
    return d[:1].upper() + d[1:]


def grambank_variables(codes, questions):
    """(id, title, question, [(code, label)]) for FEATURES."""
    by_feature = {}
    for c in codes:
        by_feature.setdefault(c["Parameter_ID"], []).append((int(c["Name"]), label(c["Description"])))
    out = []
    for fid, title in FEATURES:
        if fid not in by_feature:
            raise ValueError(f"Grambank has no codes for {fid}")
        out.append((fid, title, questions.get(fid, ""), sorted(by_feature[fid])))
    return out


def grambank_codes(values, languages):
    """{(glottocode, feature): code} at the language level."""
    level_of = {r["ID"]: (r["Language_level_ID"] or r["ID"]) for r in languages}
    wanted = {f for f, _ in FEATURES}
    # (language, feature) -> (0 if the language itself, else 1; Grambank id; code)
    best = {}
    for v in values:
        f = v["Parameter_ID"]
        if f not in wanted or v["Value"] in ("", "?"):
            continue
        gid = v["Language_ID"]
        lang = level_of.get(gid, gid)
        rank = (0 if gid == lang else 1, gid)
        key = (lang, f)
        if key not in best or rank < best[key][:2]:
            best[key] = (*rank, int(v["Value"]))
    return {k: b[2] for k, b in best.items()}


def convert(languages_text, values_text, gb_values, gb_codes, gb_languages, gb_questions=None):
    rows = list(csv.DictReader(io.StringIO(languages_text)))
    names = {r["ID"]: r["Name"] for r in rows}
    aes = {}
    for v in csv.DictReader(io.StringIO(values_text)):
        if v["Parameter_ID"] == "aes" and v["Value"]:
            aes[v["Language_ID"]] = int(v["Value"])
    variables = [ENDANGERMENT] + grambank_variables(gb_codes, gb_questions or {})
    codes = grambank_codes(gb_values, gb_languages)
    known = [{c for c, _ in var[3]} for var in variables]
    lines = [MAGIC]
    for vid, title, question, pairs in variables:
        text = "|".join(f"{c}={clean(lab)}" for c, lab in pairs)
        lines.append("\t".join(["V", vid, clean(title), clean(question), text]))
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
        fields = ["L", r["ID"], r["Name"], family, f"{lat:.4f}", f"{lon:.4f}", r["ISO639P3code"], r["Macroarea"]]
        if any("\t" in f or "\n" in f for f in fields):
            raise ValueError(f"tab or newline in {fields}")
        vals = [aes.get(r["ID"])] + [codes.get((r["ID"], fid)) for fid, _ in FEATURES]
        for v, ok in zip(vals, known):
            if v is not None and v not in ok:
                raise ValueError(f"{r['ID']}: unknown code {v}")
        fields += ["" if v is None else str(v) for v in vals]
        out.append(fields)
    out.sort(key=lambda f: f[1])
    lines += ["\t".join(f) for f in out]
    return "\n".join(lines) + "\n"


def read_text(path):
    with open(path, encoding="utf-8", newline="") as f:
        return f.read()


def read_rows(path):
    return list(csv.DictReader(io.StringIO(read_text(path))))


QUESTIONS = {
    "GB020": "Are there definite or specific articles?",
    "GB030": "Is there a gender distinction in independent 3rd person pronouns?",
    "GB051": "Is there a gender/noun class system where sex is a factor in class assignment?",
    "GB057": "Are there numeral classifiers?",
    "GB028": "Is there a distinction between inclusive and exclusive?",
    "GB070": "Are there morphological cases for non-pronominal core arguments (i.e. S/A/P)?",
    "GB409": "Is there any ergative alignment of flagging?",
    "GB133": "Is a pragmatically unmarked constituent order verb-final for transitive clauses?",
    "GB131": "Is a pragmatically unmarked constituent order verb-initial for transitive clauses?",
    "GB024": "What is the order of numeral and noun in the NP?",
    "GB074": "Are there prepositions?",
    "GB075": "Are there postpositions?",
    "GB323": "Is there grammatical marking of indirect evidence (hearsay, inference, etc.)?",
    "GB415": "Is there a politeness distinction in 2nd person forms?",
    "GB335": "Is there synchronic evidence for any element of a vigesimal numeral system?",
}


def main(argv):
    if argv:
        paths = {k: os.path.join(argv[0], f) for k, f in LOCAL.items()}
    else:
        import data_sources

        paths = {k: data_sources.fetch(k) for k in LOCAL}
    text = convert(
        read_text(paths["glottolog-v54-languages"]),
        read_text(paths["glottolog-v54-values"]),
        read_rows(paths["grambank-v103-values"]),
        read_rows(paths["grambank-v103-codes"]),
        read_rows(paths["grambank-v103-languages"]),
        QUESTIONS,
    )
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    rows = [line.split("\t") for line in text.splitlines() if line.startswith("L\t")]
    print(f"wrote {OUT}: {len(rows)} languages", file=sys.stderr)
    for i, (vid, *_rest) in enumerate([ENDANGERMENT] + FEATURES):
        print(f"  {vid}: {sum(1 for r in rows if r[8 + i])} with data", file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv[1:])
