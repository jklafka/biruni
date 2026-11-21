#!/usr/bin/env python3
"""Build assets/data/history_borders.bin, the historical borders layer, from
Cliopatria (Seshat Global History Databank, CC BY 4.0), pinned in
data-sources.lock.json as `cliopatria-v021`.

The output is a HIST file (scripts/hist_format.py, docs/historical-data.md)
in calendar years: one record per Cliopatria POLITY row (relations, such as
unions, are left out because they overlap their members), valid from
FromYear to ToYear inclusive. Each ring is simplified (Douglas-Peucker, at
most TOLERANCE degrees, less for small rings so small polities keep their
shape) and coordinates are kept to 2 decimals (about 1 km). Identical geometries are
stored once.

Each record's tag packs two things the app needs:
- bits 0-2, the fill color (0..PALETTE_SIZE): a greedy coloring of the
  graph of polities that are ever neighbors at the same time, so a polity
  keeps its color across the years and neighbors rarely share one;
- bits 3-5, the label tier (0 most important) from the area.
The label point is a point deep inside the largest polygon.

Stdlib only; the output is deterministic.

    python3 scripts/convert_cliopatria.py               # fetch the pinned file, then convert
    python3 scripts/convert_cliopatria.py cliopatria.geojson.zip
"""
import io
import json
import math
import os
import sys
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hist_format as hf  # noqa: E402

SOURCE = "cliopatria-v021"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "data", "history_borders.bin")
DIGITS = 2  # 0.01 degree, about 1 km: well under the simplification
TOLERANCE = 0.08  # degrees (about 9 km; the ID raster texel is 0.088)
MIN_RING_AREA = 0.0005  # square degrees: rings smaller than about 2.5 km x 2.5 km go
PALETTE_SIZE = 6  # src/country.rs PALETTE_SIZE and globe.frag PALETTE
NEIGHBOR_CELL = 0.25  # degrees: polities with vertices in one cell are neighbors
# Area (km^2) at or above which a polity gets label tier 0, 1, ... 7.
TIER_AREAS = [5e6, 1.5e6, 5e5, 1.5e5, 5e4, 1.5e4, 5e3, 0]


def simplify(points, tol):
    """Douglas-Peucker on an open polyline; returns the kept points."""
    if len(points) < 3:
        return list(points)
    keep = [False] * len(points)
    keep[0] = keep[-1] = True
    stack = [(0, len(points) - 1)]
    while stack:
        a, b = stack.pop()
        (ax, ay), (bx, by) = points[a], points[b]
        dx, dy = bx - ax, by - ay
        norm = math.hypot(dx, dy)
        best, idx = -1.0, -1
        for i in range(a + 1, b):
            px, py = points[i]
            if norm == 0:
                d = math.hypot(px - ax, py - ay)
            else:
                d = abs(dx * (ay - py) - dy * (ax - px)) / norm
            if d > best:
                best, idx = d, i
        if idx >= 0 and best > tol:
            keep[idx] = True
            stack.append((a, idx))
            stack.append((idx, b))
    return [p for p, k in zip(points, keep) if k]


def ring_area(ring):
    s = 0.0
    for (x0, y0), (x1, y1) in zip(ring, ring[1:] + ring[:1]):
        s += x0 * y1 - x1 * y0
    return s / 2.0


def simplify_ring(ring):
    """A closed ring (first point not repeated) simplified, or None if too
    small to keep."""
    if len(ring) > 1 and ring[0] == ring[-1]:
        ring = ring[:-1]
    if len(ring) < 3:
        return None
    xs, ys = [p[0] for p in ring], [p[1] for p in ring]
    size = max(max(xs) - min(xs), max(ys) - min(ys))
    tol = min(TOLERANCE, size / 10.0)
    # Split the closed ring at its farthest point from the first, so both
    # halves are open polylines with fixed ends.
    far = max(range(len(ring)), key=lambda i: (ring[i][0] - ring[0][0]) ** 2 + (ring[i][1] - ring[0][1]) ** 2)
    if far == 0:
        return None
    a = simplify(ring[: far + 1], tol)
    b = simplify(ring[far:] + ring[:1], tol)
    out = a[:-1] + b[:-1]
    q = 10 ** DIGITS
    out = [(round(x * q) / q, round(y * q) / q) for x, y in out]
    # Drop repeats made by rounding.
    dedup = [p for i, p in enumerate(out) if p != out[i - 1]] if len(out) > 1 else out
    if len(dedup) < 3 or abs(ring_area(dedup)) < MIN_RING_AREA:
        return None
    return dedup


def geometry_rings(geom):
    t, c = geom["type"], geom["coordinates"]
    if t == "Polygon":
        polys = [c]
    elif t == "MultiPolygon":
        polys = c
    else:
        return []
    return [[(float(x), float(y)) for x, y, *_ in ring] for poly in polys for ring in poly]


def inside(rings, x, y):
    """Even-odd point in polygon over all rings."""
    c = False
    for ring in rings:
        j = len(ring) - 1
        for i in range(len(ring)):
            (xi, yi), (xj, yj) = ring[i], ring[j]
            if (yi > y) != (yj > y) and x < (xj - xi) * (y - yi) / (yj - yi) + xi:
                c = not c
            j = i
    return c


def edge_distance(rings, x, y):
    best = float("inf")
    for ring in rings:
        for (ax, ay), (bx, by) in zip(ring, ring[1:] + ring[:1]):
            dx, dy = bx - ax, by - ay
            L = dx * dx + dy * dy
            t = 0.0 if L == 0 else max(0.0, min(1.0, ((x - ax) * dx + (y - ay) * dy) / L))
            best = min(best, math.hypot(x - ax - t * dx, y - ay - t * dy))
    return best


def label_point(rings):
    """A point well inside the biggest ring's area (grid search, then a
    finer search around the best), on the even-odd fill of all rings."""
    big = max(rings, key=lambda r: abs(ring_area(r)))
    xs, ys = [p[0] for p in big], [p[1] for p in big]
    x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)
    best, bx, by = -1.0, (x0 + x1) / 2, (y0 + y1) / 2
    near = [r for r in rings if not (max(p[0] for p in r) < x0 or min(p[0] for p in r) > x1
                                     or max(p[1] for p in r) < y0 or min(p[1] for p in r) > y1)]
    w, h = x1 - x0, y1 - y0
    for step in range(2):
        n = 12
        cx, cy = (bx, by) if step else ((x0 + x1) / 2, (y0 + y1) / 2)
        sw, sh = (w, h) if step == 0 else (w / 6, h / 6)
        for i in range(n + 1):
            for j in range(n + 1):
                x = cx - sw / 2 + sw * i / n
                y = cy - sh / 2 + sh * j / n
                if inside(near, x, y):
                    d = edge_distance(near, x, y)
                    if d > best:
                        best, bx, by = d, x, y
    if best < 0:
        bx, by = big[0]
    q = 10 ** DIGITS
    return (round(bx * q) / q, round(by * q) / q)


def tier(area):
    for i, a in enumerate(TIER_AREAS):
        if area >= a:
            return i
    return 7


def colors(names, rows):
    """Greedy coloring of the 'neighbors at the same time' graph of names:
    most-connected names first, each taking the color least used by its
    already colored neighbors (ties to the lowest index)."""
    cells = {}
    for idx, (name, start, end, rings) in enumerate(rows):
        seen = set()
        for ring in rings:
            for x, y in ring:
                seen.add((math.floor(x / NEIGHBOR_CELL), math.floor(y / NEIGHBOR_CELL)))
        for c in seen:
            cells.setdefault(c, []).append(idx)
    adj = {n: set() for n in range(len(names))}
    for members in cells.values():
        for i in range(len(members)):
            a = rows[members[i]]
            for k in range(i + 1, len(members)):
                b = rows[members[k]]
                if a[0] != b[0] and a[1] <= b[2] and b[1] <= a[2]:
                    adj[a[0]].add(b[0])
                    adj[b[0]].add(a[0])
    order = sorted(adj, key=lambda n: (-len(adj[n]), names[n]))
    color = {}
    for n in order:
        used = [0] * PALETTE_SIZE
        for m in adj[n]:
            if m in color:
                used[color[m]] += 1
        color[n] = min(range(PALETTE_SIZE), key=lambda c: (used[c], c))
    return color


def convert(features):
    """HIST bytes from Cliopatria GeoJSON features."""
    polities = [f for f in features if f["properties"].get("Type") == "POLITY"]
    names = sorted({f["properties"]["Name"] for f in polities})
    name_index = {n: i for i, n in enumerate(names)}
    rows = []
    for f in polities:
        p = f["properties"]
        rings = [r for r in (simplify_ring(r) for r in geometry_rings(f["geometry"])) if r]
        if not rings:
            continue
        rows.append((name_index[p["Name"]], int(p["FromYear"]), int(p["ToYear"]), rings, float(p.get("Area") or 0)))
    rows.sort(key=lambda r: (r[1], r[2], names[r[0]]))
    color = colors(names, [r[:4] for r in rows])
    geometries, geom_index, labels, records = [], {}, [], []
    for name, start, end, rings, area in rows:
        key = tuple(tuple(r) for r in rings)
        if key not in geom_index:
            geom_index[key] = len(geometries)
            geometries.append(rings)
            labels.append(label_point(rings))
        g = geom_index[key]
        tag = color[name] | (tier(area) << 3)
        records.append((name, start, end, g, labels[g], tag))
    # Names no row kept (all rings too small) still get an index; harmless.
    return hf.encode(hf.SCALE_YEARS, DIGITS, names, geometries, records)


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
    data = convert(load_features(src))
    with open(OUT, "wb") as f:
        f.write(data)
    print(f"wrote {OUT}: {len(data)} bytes", file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv[1:])
