#!/usr/bin/env python3
"""Build the deep time view's data from the Merdith et al. (2021) plate model
v1.2.4 (EarthByte, CC BY 4.0), pinned in data-sources.lock.json as
`merdith2021-v124`:

- assets/data/paleo_coastlines.bin: the model's present-day coastline
  polygons, a HIST file (scripts/hist_format.py) in geological time: each
  record holds a polygon from the age it appears to the age it ends (or the
  present), tagged with the plate id that carries it. Rings are simplified
  (Douglas-Peucker, TOLERANCE degrees) and kept to 2 decimals.
- assets/data/paleo_rotations.tsv: the rotation model, one finite rotation
  per line (src/paleo.rs reads it):

      MAPSROT1
      <moving plate>  <age Ma>  <pole lat>  <pole lon>  <angle deg>  <fixed plate>

  in file order, without the commented-out plate 999 lines.

The browser composes the rotations to reconstruct the coastlines at any age.
Stdlib only; deterministic.

    python3 scripts/convert_merdith.py            # fetch the pinned zip, then convert
    python3 scripts/convert_merdith.py model.zip
"""
import gzip
import io
import os
import sys
import xml.etree.ElementTree as ET
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hist_format as hf  # noqa: E402
from convert_cliopatria import ring_area, simplify  # noqa: E402

SOURCE = "merdith2021-v124"
HERE = os.path.dirname(os.path.abspath(__file__))
OUT_BIN = os.path.join(HERE, "..", "assets", "data", "paleo_coastlines.bin")
OUT_ROT = os.path.join(HERE, "..", "assets", "data", "paleo_rotations.tsv")
COASTLINES = "Coastlines/shapes_coastlines_Merdith_et_al_v2.gpmlz"
ROTATIONS = "Rotations/1000_0_rotfile_Merdith_etal.rot"
DIGITS = 2
TOLERANCE = 0.1  # degrees
MIN_RING_AREA = 0.01  # square degrees
OLDEST = 1000.0  # Ma: the model's span; older features start here

NS = {
    "gml": "http://www.opengis.net/gml",
    "gpml": "http://www.gplates.org/gplates",
}


def age(text):
    """An age in Ma from a gml:timePosition ('distantPast' is the oldest,
    'distantFuture' the present)."""
    t = (text or "").strip()
    if "distantPast" in t:
        return OLDEST
    if "distantFuture" in t:
        return 0.0
    return min(float(t), OLDEST)


def rings_of(feature):
    """Rings (lon, lat) of a feature's polygons: exterior and interiors."""
    out = []
    for pos in feature.iter(f"{{{NS['gml']}}}posList"):
        v = [float(x) for x in pos.text.split()]
        # GPML lists latitude first.
        out.append([(v[i + 1], v[i]) for i in range(0, len(v) - 1, 2)])
    return out


def simplify_ring(ring):
    if len(ring) > 1 and ring[0] == ring[-1]:
        ring = ring[:-1]
    if len(ring) < 3:
        return None
    far = max(range(len(ring)), key=lambda i: (ring[i][0] - ring[0][0]) ** 2 + (ring[i][1] - ring[0][1]) ** 2)
    if far == 0:
        return None
    pts = simplify(ring[: far + 1], TOLERANCE)[:-1] + simplify(ring[far:] + ring[:1], TOLERANCE)[:-1]
    q = 10 ** DIGITS
    pts = [(round(x * q) / q, round(y * q) / q) for x, y in pts]
    pts = [p for i, p in enumerate(pts) if p != pts[i - 1]] if len(pts) > 1 else pts
    if len(pts) < 3 or abs(ring_area(pts)) < MIN_RING_AREA:
        return None
    return pts


def coastlines(xml_bytes):
    """HIST bytes from the coastline GPML."""
    root = ET.fromstring(xml_bytes)
    names, name_index, rows = [], {}, []
    for member in root.iter(f"{{{NS['gml']}}}featureMember"):
        feature = next(iter(member), None)
        if feature is None:
            continue
        pid_el = feature.find("gpml:reconstructionPlateId/gpml:ConstantValue/gpml:value", NS)
        begin = feature.find("gml:validTime/gml:TimePeriod/gml:begin/gml:TimeInstant/gml:timePosition", NS)
        end = feature.find("gml:validTime/gml:TimePeriod/gml:end/gml:TimeInstant/gml:timePosition", NS)
        if pid_el is None or begin is None or end is None:
            continue
        old, young = age(begin.text), age(end.text)
        if old < young:
            continue
        rings = [r for r in (simplify_ring(r) for r in rings_of(feature)) if r]
        if not rings:
            continue
        name_el = feature.find("gml:name", NS)
        name = (name_el.text or "").strip() if name_el is not None else ""
        if name not in name_index:
            name_index[name] = len(names)
            names.append(name)
        # Later is larger: -(age in tenths of Ma).
        rows.append((name_index[name], -round(old * 10), -round(young * 10), rings, int(pid_el.text)))
    rows.sort(key=lambda r: (r[1], r[2], r[4], r[3][0][0]))
    geometries = [r[3] for r in rows]
    records = [(n, start, stop, i, (0.0, 0.0), pid) for i, (n, start, stop, _, pid) in enumerate(rows)]
    return hf.encode(hf.SCALE_MA, DIGITS, names, geometries, records)


def rotations(text):
    """The rotation table, without comment lines and plate 999."""
    out = ["MAPSROT1"]
    for line in text.splitlines():
        body = line.split("!", 1)[0].split()
        if len(body) < 6 or body[0] == "999":
            continue
        moving, age_, lat, lon, angle, fixed = body[:6]
        out.append("\t".join([str(int(moving)), age_, lat, lon, angle, str(int(fixed))]))
    return "\n".join(out) + "\n"


def main(argv):
    if argv:
        src = argv[0]
    else:
        import data_sources

        src = data_sources.fetch(SOURCE)
    with zipfile.ZipFile(src) as z:
        coast = gzip.decompress(z.read(COASTLINES))
        rot = z.read(ROTATIONS).decode("utf-8", errors="replace")
    data = coastlines(coast)
    with open(OUT_BIN, "wb") as f:
        f.write(data)
    text = rotations(rot)
    with open(OUT_ROT, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"wrote {OUT_BIN}: {len(data)} bytes; {OUT_ROT}: {text.count(chr(10)) - 1} rotations", file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv[1:])
