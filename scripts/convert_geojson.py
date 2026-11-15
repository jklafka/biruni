#!/usr/bin/env python3
"""Convert Natural Earth GeoJSON into the compact `.bin` format read by
`geo::decode_shapes` (see the format description in src/geo.rs).

Usage: python3 scripts/convert_geojson.py [data-src [assets/data]]

Mirrors `geo::parse_shapes`: points are skipped, positions with fewer than two
coordinates are dropped, shapes with fewer than two points are discarded.
`ne_50m_admin_0_countries.geojson` is converted to the separate country format
(`MAPC`, see src/country.rs): polygons with a small attribute table.
`ne_10m_populated_places_simple.geojson` is not a map layer; it and the
countries file feed `places.tsv`, the search index read by src/search.rs
(see `build_places` below for the format). The same two files plus
`ne_50m_marine_label_points.geojson` (made by `marine_label_points.py`) feed
`labels.tsv`, read by src/labels.rs (see `build_labels`).

Output is deterministic, so rerunning it must not change the committed files.
"""
import json
import math
import sys
from pathlib import Path

MAGIC = b"MAPD"
VERSION = 1
SCALE = 10_000  # units per degree (1e-4 degree, about 11 m)


def varint(n: int) -> bytes:
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def zigzag(n: int) -> int:
    return (n << 1) ^ (n >> 63)


def shapes_of(geojson):
    def ring(coords, closed):
        pts = [(p[0], p[1]) for p in coords if len(p) >= 2]
        return [(pts, closed)] if len(pts) >= 2 else []

    for f in geojson["features"]:
        g = f.get("geometry")
        if not g:
            continue
        t, c = g["type"], g["coordinates"]
        if t == "LineString":
            yield from ring(c, False)
        elif t == "MultiLineString":
            for line in c:
                yield from ring(line, False)
        elif t == "Polygon":
            for r in c:
                yield from ring(r, True)
        elif t == "MultiPolygon":
            for poly in c:
                for r in poly:
                    yield from ring(r, True)


def encode(geojson) -> bytes:
    shapes = list(shapes_of(geojson))
    out = bytearray(MAGIC + bytes([VERSION]) + varint(len(shapes)))
    px = py = 0
    for pts, closed in shapes:
        out += varint(len(pts) << 1 | int(closed))
        for lon, lat in pts:
            x, y = round(lon * SCALE), round(lat * SCALE)
            out += varint(zigzag(x - px)) + varint(zigzag(y - py))
            px, py = x, y
    return bytes(out)


COUNTRY_MAGIC = b"MAPC"
COUNTRY_VERSION = 2
PALETTE_SIZE = 6  # political fill colors; keep in sync with PALETTE in src/shaders/globe.frag
COUNTRY_SCALE = 1_000  # units per degree (1e-3 degree, about 110 m)


def string(s: str) -> bytes:
    raw = s.encode("utf-8")
    return varint(len(raw)) + raw


def clean_code(*values) -> str:
    """First usable code; Natural Earth uses -99 for 'none'."""
    for v in values:
        if isinstance(v, str) and v and v != "-99":
            return v
    return ""


def country_adjacency(countries):
    """Sets of country indexes that share a border, from the converted polygons.

    Two countries are neighbors if they have a boundary edge in common: the same
    pair of vertices, after rounding to COUNTRY_SCALE as the file stores them.
    Natural Earth draws shared borders with identical vertices, so this finds
    land borders without treating a point contact (four corners) as adjacency.
    """
    owners = {}
    for ci, (_, polys) in enumerate(countries):
        for rings in polys:
            for pts in rings:
                q = [(round(x * COUNTRY_SCALE), round(y * COUNTRY_SCALE)) for x, y in pts]
                for a, b in zip(q, q[1:] + q[:1]):
                    if a != b:
                        owners.setdefault((min(a, b), max(a, b)), set()).add(ci)
    adj = [set() for _ in countries]
    for cs in owners.values():
        for a in cs:
            adj[a] |= cs - {a}
    return adj


def color_countries(adj):
    """DSatur graph coloring with PALETTE_SIZE colors; deterministic. Among the
    allowed colors it takes the least used so far, which keeps the palette
    balanced. Raises if a country cannot be colored."""
    n = len(adj)
    color = [-1] * n
    used = [0] * PALETTE_SIZE
    for _ in range(n):
        def key(i):
            seen = {color[j] for j in adj[i] if color[j] >= 0}
            return (-len(seen), -len(adj[i]), i)
        i = min((i for i in range(n) if color[i] < 0), key=key)
        taken = {color[j] for j in adj[i] if color[j] >= 0}
        free = [c for c in range(PALETTE_SIZE) if c not in taken]
        if not free:
            raise ValueError(f"country {i} needs more than {PALETTE_SIZE} colors")
        color[i] = min(free, key=lambda c: (used[c], c))
        used[color[i]] += 1
    return color


def encode_countries(geojson) -> bytes:
    """Country polygons and attributes. Rings drop their repeated closing point
    (they are implicitly closed) and rings with fewer than 3 points are dropped.
    Deltas run across the whole file, in COUNTRY_SCALE units."""
    countries = []
    for f in geojson["features"]:
        g = f.get("geometry")
        if not g or g["type"] not in ("Polygon", "MultiPolygon"):
            continue
        polys = g["coordinates"] if g["type"] == "MultiPolygon" else [g["coordinates"]]
        out_polys = []
        for poly in polys:
            rings = []
            for r in poly:
                pts = [(p[0], p[1]) for p in r if len(p) >= 2]
                if len(pts) > 1 and pts[0] == pts[-1]:
                    pts.pop()
                if len(pts) >= 3:
                    rings.append(pts)
            if rings:
                out_polys.append(rings)
        if out_polys:
            countries.append((f["properties"], out_polys))
    colors = color_countries(country_adjacency(countries))
    out = bytearray(COUNTRY_MAGIC + bytes([COUNTRY_VERSION]))
    out += varint(COUNTRY_SCALE) + varint(len(countries))
    px = py = 0
    for (props, polys), color in zip(countries, colors):
        out += string(props["NAME"])
        out += string(clean_code(props.get("ISO_A2_EH"), props.get("ISO_A2")))
        out += string(clean_code(props.get("ISO_A3_EH"), props.get("ADM0_A3")))
        out += string(props.get("CONTINENT") or "")
        out += string(props.get("SUBREGION") or "")
        out += varint(max(int(props.get("POP_EST") or 0), 0))
        out += bytes([color])
        out += varint(len(polys))
        for rings in polys:
            out += varint(len(rings))
            for pts in rings:
                out += varint(len(pts))
                for lon, lat in pts:
                    x, y = round(lon * COUNTRY_SCALE), round(lat * COUNTRY_SCALE)
                    out += varint(zigzag(x - px)) + varint(zigzag(y - py))
                    px, py = x, y
    return bytes(out)


# Sources that feed the place-search index instead of the map layers.
PLACES_SRC = "ne_10m_populated_places_simple.geojson"
COUNTRIES_SRC = "ne_50m_admin_0_countries.geojson"
PLACES_MAGIC = "MAPSPLACES1"
MARINE_SRC = "ne_50m_marine_label_points.geojson"
LABELS_MAGIC = "MAPSLABELS1"
LABEL_MIN_POP = 100_000  # smaller places are labeled only if they are a national capital


def _clean(s) -> str:
    return " ".join(str(s or "").replace("\t", " ").replace("\n", " ").split())


def _largest_polygon_box(geom):
    """(lon0, lat0, lon1, lat1) of the polygon with the largest bounding box.

    A country's full box is useless for countries that straddle the 180th
    meridian or own far-flung islands (Russia, USA, France, Fiji), so the
    mainland, taken as the biggest polygon, defines where to fly.
    """
    polys = [geom["coordinates"]] if geom["type"] == "Polygon" else geom["coordinates"]
    best = None
    for poly in polys:
        pts = poly[0]
        lons, lats = [p[0] for p in pts], [p[1] for p in pts]
        box = (min(lons), min(lats), max(lons), max(lats))
        area = (box[2] - box[0]) * (box[3] - box[1])
        if best is None or area > best[0]:
            best = (area, box)
    return best[1]


def build_places(src: Path) -> bytes:
    """Search index: a header line, then one tab-separated line per entry:

        kind  name  country  lat  lon  population  extent  aliases

    kind is C (country) or P (populated place); lat/lon are degrees with two
    decimals (about 1 km); extent is the angular size in degrees of the area
    to frame (0 for places); aliases are `;`-separated alternate names.
    Entries are sorted by population, descending, then name.
    """
    rows = []
    for f in json.loads((src / COUNTRIES_SRC).read_text(encoding="utf-8"))["features"]:
        p, g = f["properties"], f.get("geometry")
        if not g:
            continue
        lon0, lat0, lon1, lat1 = _largest_polygon_box(g)
        clat, clon = (lat0 + lat1) / 2, (lon0 + lon1) / 2
        extent = max(lat1 - lat0, (lon1 - lon0) * math.cos(math.radians(clat)))
        name = _clean(p["NAME"])
        aliases = []
        for k in ("NAME_LONG", "ADMIN"):
            a = _clean(p.get(k))
            if a and a != name and a not in aliases:
                aliases.append(a)
        rows.append(("C", name, "", clat, clon, max(0, int(p.get("POP_EST") or 0)), extent, aliases))
    for f in json.loads((src / PLACES_SRC).read_text(encoding="utf-8"))["features"]:
        p = f["properties"]
        name = _clean(p["name"])
        aliases = [a for a in (_clean(x) for x in (p.get("namealt") or "").split(",")) if a and a != name]
        rows.append(("P", name, _clean(p["adm0name"]), p["latitude"], p["longitude"],
                     max(0, int(p.get("pop_max") or 0)), 0.0, aliases))
    rows.sort(key=lambda r: (-r[5], r[1], r[0]))
    lines = [PLACES_MAGIC]
    for kind, name, country, lat, lon, pop, extent, aliases in rows:
        lines.append("\t".join([kind, name, country, f"{lat:.2f}", f"{lon:.2f}", str(pop),
                                f"{extent:.1f}", ";".join(aliases)]))
    return ("\n".join(lines) + "\n").encode("utf-8")


def _city_tier(pop: int, capital: bool) -> int:
    """0 (largest) to 7. Capitals count at least as tier 4 so small states show."""
    tier = 7
    for t, floor in enumerate((10_000_000, 5_000_000, 2_000_000, 1_000_000, 500_000, 250_000, 100_000)):
        if pop >= floor:
            tier = t
            break
    return min(tier, 4) if capital else tier


def build_labels(src: Path) -> bytes:
    """Map labels: a header line, then one tab-separated line per label:

        kind  name  lat  lon  tier  population  extent

    kind is C (country), P (populated place) or O (ocean or sea); lat/lon are
    degrees with two decimals; tier is 0 (most important) to 7 and sets how far
    in a place label appears (oceans and seas use Natural Earth's scalerank);
    extent is the angular size in degrees (0 for places). Countries are placed at
    Natural Earth's label point. Populated places are the national capitals
    and everything with at least LABEL_MIN_POP people. Sorted by kind then
    population, descending, so output is deterministic.
    """
    rows = []
    for f in json.loads((src / COUNTRIES_SRC).read_text(encoding="utf-8"))["features"]:
        p, g = f["properties"], f.get("geometry")
        if not g:
            continue
        lon0, lat0, lon1, lat1 = _largest_polygon_box(g)
        extent = max(lat1 - lat0, (lon1 - lon0) * math.cos(math.radians((lat0 + lat1) / 2)))
        rows.append(("C", _clean(p["NAME"]), p["LABEL_Y"], p["LABEL_X"], 0, max(0, int(p.get("POP_EST") or 0)), extent))
    for f in json.loads((src / PLACES_SRC).read_text(encoding="utf-8"))["features"]:
        p = f["properties"]
        pop, capital = max(0, int(p.get("pop_max") or 0)), p.get("adm0cap") == 1
        if pop < LABEL_MIN_POP and not capital:
            continue
        rows.append(("P", _clean(p["name"]), p["latitude"], p["longitude"], _city_tier(pop, capital), pop, 0.0))
    for f in json.loads((src / MARINE_SRC).read_text(encoding="utf-8"))["features"]:
        p, (lon, lat) = f["properties"], f["geometry"]["coordinates"]
        rows.append(("O", _clean(p["name"]), lat, lon, int(p["scalerank"]), 0, p["extent"]))
    rows.sort(key=lambda r: ("COP".index(r[0]), r[4], -r[5], r[1]))
    lines = [LABELS_MAGIC]
    for kind, name, lat, lon, tier, pop, extent in rows:
        lines.append("\t".join([kind, name, f"{lat:.2f}", f"{lon:.2f}", str(tier), str(pop), f"{extent:.1f}"]))
    return ("\n".join(lines) + "\n").encode("utf-8")


def main():
    src = Path(sys.argv[1] if len(sys.argv) > 1 else "data-src")
    dst = Path(sys.argv[2] if len(sys.argv) > 2 else "assets/data")
    dst.mkdir(parents=True, exist_ok=True)
    for f in sorted(src.glob("*.geojson")):
        if f.name in (PLACES_SRC, MARINE_SRC):
            continue
        gj = json.loads(f.read_text(encoding="utf-8"))
        data = encode_countries(gj) if "admin_0_countries" in f.stem else encode(gj)
        (dst / (f.stem + ".bin")).write_bytes(data)
        print(f"{f.name}: {f.stat().st_size} -> {len(data)} bytes")
    if (src / PLACES_SRC).exists() and (src / COUNTRIES_SRC).exists():
        data = build_places(src)
        (dst / "places.tsv").write_bytes(data)
        print(f"places.tsv: {len(data)} bytes")
        if (src / MARINE_SRC).exists():
            data = build_labels(src)
            (dst / "labels.tsv").write_bytes(data)
            print(f"labels.tsv: {len(data)} bytes")


if __name__ == "__main__":
    main()
