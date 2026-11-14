#!/usr/bin/env python3
"""Derive one label point per ocean or sea from Natural Earth marine polygons.

Natural Earth's `ne_50m_geography_marine_polys` (public domain) has polygons
and names but no label points. This reads the shapefile with the standard
library only and writes `data-src/ne_50m_marine_label_points.geojson`: for each
named feature, the point inside its largest ring that is farthest from the ring's
edge (a coarse "pole of inaccessibility", searched on a grid in lon/lat), plus
the feature's size in degrees.

Run it only when updating the source; the output is committed and
`convert_geojson.py` builds the app's `labels.tsv` from it.

    curl -LO https://naturalearth.s3.amazonaws.com/50m_physical/ne_50m_geography_marine_polys.zip
    unzip ne_50m_geography_marine_polys.zip -d marine
    python3 scripts/marine_label_points.py marine/ne_50m_geography_marine_polys
"""
import json
import math
import struct
import sys
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "data-src" / "ne_50m_marine_label_points.geojson"


def read_dbf(path):
    d = Path(path).read_bytes()
    n, hl, rl = struct.unpack("<xxxxIHH", d[:12])
    fields, o = [], 32
    while d[o] != 0x0D:
        fields.append((d[o:o + 11].split(b"\0")[0].decode(), d[o + 16]))
        o += 32
    rows = []
    for i in range(n):
        r, p, row = d[hl + i * rl:hl + (i + 1) * rl], 1, {}
        for name, ln in fields:
            row[name] = r[p:p + ln].decode("utf-8", "replace").strip()
            p += ln
        rows.append(row)
    return rows


def read_shp(path):
    d = Path(path).read_bytes()
    pos, out = 100, []
    while pos < len(d):
        (length,) = struct.unpack(">I", d[pos + 4:pos + 8])
        body = d[pos + 8:pos + 8 + length * 2]
        pos += 8 + length * 2
        (stype,) = struct.unpack("<i", body[:4])
        if stype == 0:
            out.append([])
            continue
        nparts, npts = struct.unpack("<ii", body[36:44])
        parts = list(struct.unpack(f"<{nparts}i", body[44:44 + 4 * nparts]))
        o = 44 + 4 * nparts
        pts = [struct.unpack("<dd", body[o + 16 * i:o + 16 * i + 16]) for i in range(npts)]
        parts.append(npts)
        out.append([pts[parts[i]:parts[i + 1]] for i in range(nparts)])
    return out


def ring_area(r):
    return 0.5 * sum(r[i][0] * r[i + 1][1] - r[i + 1][0] * r[i][1] for i in range(len(r) - 1))


def inside(r, x, y):
    c = False
    for i in range(len(r) - 1):
        (x0, y0), (x1, y1) = r[i], r[i + 1]
        if (y0 > y) != (y1 > y) and x < (x1 - x0) * (y - y0) / (y1 - y0) + x0:
            c = not c
    return c


def edge_dist(r, x, y):
    best = 1e18
    for i in range(len(r) - 1):
        (x0, y0), (x1, y1) = r[i], r[i + 1]
        dx, dy = x1 - x0, y1 - y0
        t = 0.0 if dx == dy == 0 else max(0.0, min(1.0, ((x - x0) * dx + (y - y0) * dy) / (dx * dx + dy * dy)))
        best = min(best, (x - x0 - t * dx) ** 2 + (y - y0 - t * dy) ** 2)
    return best


def label_point(r):
    xs, ys = [p[0] for p in r], [p[1] for p in r]
    x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)
    # Thin the ring for the distance search; this only needs to be roughly right.
    step = max(1, len(r) // 400)
    thin = r[::step] + [r[0]]
    best, n = None, 40
    for i in range(1, n):
        for j in range(1, n):
            x, y = x0 + (x1 - x0) * i / n, y0 + (y1 - y0) * j / n
            if inside(r, x, y):
                d = edge_dist(thin, x, y)
                if best is None or d > best[0]:
                    best = (d, x, y)
    if best is None:
        return (x0 + x1) / 2, (y0 + y1) / 2, 0.0
    cy = (y0 + y1) / 2
    return best[1], best[2], max(y1 - y0, (x1 - x0) * math.cos(math.radians(cy)))


def title(name):
    return name.title() if name.isupper() else name


def main():
    base = sys.argv[1]
    rows, shapes = read_dbf(base + ".dbf"), read_shp(base + ".shp")
    feats, seen = [], {}
    for row, rings in zip(rows, shapes):
        if not rings or not row["name"]:
            continue
        ring = max(rings, key=lambda r: abs(ring_area(r)))
        lon, lat, extent = label_point(ring)
        name = title(row["name"])
        f = {
            "type": "Feature",
            "properties": {
                "name": name,
                "featurecla": row["featurecla"],
                "scalerank": int(row["scalerank"] or 0),
                "extent": round(extent, 1),
            },
            "geometry": {"type": "Point", "coordinates": [round(lon, 2), round(lat, 2)]},
        }
        # A name can appear on several polygons; keep the largest.
        if name in seen and feats[seen[name]]["properties"]["extent"] >= f["properties"]["extent"]:
            continue
        if name in seen:
            feats[seen[name]] = f
        else:
            seen[name] = len(feats)
            feats.append(f)
    OUT.write_text(json.dumps({"type": "FeatureCollection", "features": feats}, ensure_ascii=False, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"{len(feats)} label points -> {OUT}")


if __name__ == "__main__":
    main()
