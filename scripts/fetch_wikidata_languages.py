#!/usr/bin/env python3
"""Fetch data-src/wikidata_polity_languages.tsv: the languages Wikidata gives
for the historical borders' polities, which `convert_seshat_languages.py`
uses where Seshat has none (most polities after 1900, and many before).

Wikidata is CC0. Every Cliopatria polity (`cliopatria-v021`) names its
Wikidata item; this asks the Wikidata Query Service, in batches of BATCH
items one request at a time with a pause between them, for each item's
official languages (P37) and, as a fallback the converter uses only when
there are none, its languages used (P2936), with each language's English
label and Glottolog code (P1394; for a language without one, the code of
a language it is a subclass of (P279), or failing that of one two steps
up (Putonghua is a subclass of Standard Chinese, itself of Beijing
Mandarin), the lowest if several). The browser never asks Wikidata itself;
the committed file is a snapshot (the date is in its header), refreshed
by hand. Stdlib only.

    python3 scripts/fetch_wikidata_languages.py                    # fetch the pinned Cliopatria, then query
    python3 scripts/fetch_wikidata_languages.py CLIOPATRIA.zip

Format (UTF-8, tab-separated, sorted; read by convert_seshat_languages.py):

    MAPSWDLANG1  <snapshot date, YYYY-MM-DD>
    <polity item>  <P37 or P2936>  <language item>  <glottocode or empty>  <English label>
"""
import datetime
import io
import json
import os
import sys
import time
import urllib.parse
import urllib.request
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

MAGIC = "MAPSWDLANG1"
SOURCE = "cliopatria-v021"
ENDPOINT = "https://query.wikidata.org/sparql"
USER_AGENT = "map-data-fetch/1.0 (python-urllib; one-off batch script)"
BATCH = 150
PAUSE = 2.0  # seconds between requests
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "data-src", "wikidata_polity_languages.tsv")

QUERY = """SELECT ?polity ?prop ?lang ?glotto ?parentGlotto ?grandGlotto ?label WHERE {
  VALUES ?polity { %s }
  VALUES (?p ?prop) { (wdt:P37 "P37") (wdt:P2936 "P2936") }
  ?polity ?p ?lang .
  OPTIONAL { ?lang wdt:P1394 ?glotto }
  OPTIONAL { ?lang wdt:P279 ?parent . ?parent wdt:P1394 ?parentGlotto }
  OPTIONAL { ?lang wdt:P279/wdt:P279 ?grand . ?grand wdt:P1394 ?grandGlotto }
  OPTIONAL { ?lang rdfs:label ?label FILTER(LANG(?label) = "en") }
}"""


def polity_items(features):
    """The Wikidata items of Cliopatria's polities, sorted."""
    items = set()
    for f in features:
        p = f["properties"]
        q = (p.get("Wikidata") or "").strip()
        if p.get("Type") == "POLITY" and q.startswith("Q") and q[1:].isdigit():
            items.add(q)
    return sorted(items, key=lambda q: int(q[1:]))


def rows_from_bindings(bindings):
    """(polity, prop, language, glottocode, label) rows from SPARQL JSON:
    one per polity, property and language, with the language's own
    Glottolog codes, else the lowest of its parents', else the lowest of
    its grandparents'."""
    own, parents, grands, labels = {}, {}, {}, {}
    for b in bindings:
        def get(k):
            v = b.get(k, {}).get("value", "")
            return v.rsplit("/", 1)[-1] if v.startswith("http") else v
        key = (get("polity"), get("prop"), get("lang"))
        labels.setdefault(key, set()).add(get("label").replace("\t", " ").replace("\n", " "))
        for k, store in (("glotto", own), ("parentGlotto", parents), ("grandGlotto", grands)):
            if get(k):
                store.setdefault(key, set()).add(get(k))
    out = set()
    for key, labs in labels.items():
        if key in own:
            codes = own[key]
        elif key in parents:
            codes = {min(parents[key])}
        elif key in grands:
            codes = {min(grands[key])}
        else:
            codes = {""}
        label = min(labs)
        for code in codes:
            out.add((*key, code, label))
    return out


def query(items):
    q = QUERY % " ".join(f"wd:{i}" for i in items)
    data = urllib.parse.urlencode({"query": q, "format": "json"}).encode()
    req = urllib.request.Request(
        ENDPOINT,
        data=data,
        headers={"User-Agent": USER_AGENT, "Accept": "application/sparql-results+json"},
    )
    for attempt in range(4):
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                return json.load(r)["results"]["bindings"]
        except (OSError, ValueError) as e:
            if attempt == 3:
                raise
            print(f"  retrying after {e}", file=sys.stderr)
            time.sleep(10 * (attempt + 1))
    return []


def format_rows(rows, date):
    lines = [f"{MAGIC}\t{date}"]
    def key(r):
        return (int(r[0][1:]), r[1], int(r[2][1:]) if r[2][1:].isdigit() else 0, r[2], r[3], r[4])

    for r in sorted(rows, key=key):
        lines.append("\t".join(r))
    return "\n".join(lines) + "\n"


def parse(text):
    """{polity item: [(prop, language item, glottocode, label)]} from the file."""
    lines = text.splitlines()
    if not lines or lines[0].split("\t")[0] != MAGIC:
        raise ValueError("not a Wikidata languages file")
    out = {}
    for line in lines[1:]:
        f = line.split("\t")
        if len(f) != 5:
            raise ValueError(f"bad line {line!r}")
        out.setdefault(f[0], []).append(tuple(f[1:]))
    return out


def load_features(path):
    with zipfile.ZipFile(path) as z:
        member = next(n for n in sorted(z.namelist()) if n.endswith(".geojson"))
        with z.open(member) as f:
            return json.load(io.TextIOWrapper(f, encoding="utf-8"))["features"]


def main(argv):
    if argv:
        src = argv[0]
    else:
        import data_sources

        src = data_sources.fetch(SOURCE)
    items = polity_items(load_features(src))
    rows = set()
    for i in range(0, len(items), BATCH):
        batch = items[i : i + BATCH]
        print(f"items {i + 1}-{i + len(batch)} of {len(items)}", file=sys.stderr)
        rows |= rows_from_bindings(query(batch))
        time.sleep(PAUSE)
    date = datetime.datetime.now(datetime.timezone.utc).date().isoformat()
    text = format_rows(rows, date)
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"wrote {OUT}: {len(rows)} rows for {len({r[0] for r in rows})} of {len(items)} items", file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv[1:])
