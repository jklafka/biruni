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
(see `build_places` below for the format).

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
COUNTRY_VERSION = 1
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
    out = bytearray(COUNTRY_MAGIC + bytes([COUNTRY_VERSION]))
    out += varint(COUNTRY_SCALE) + varint(len(countries))
    px = py = 0
    for props, polys in countries:
        out += string(props["NAME"])
        out += string(clean_code(props.get("ISO_A2_EH"), props.get("ISO_A2")))
        out += string(clean_code(props.get("ISO_A3_EH"), props.get("ADM0_A3")))
        out += string(props.get("CONTINENT") or "")
        out += string(props.get("SUBREGION") or "")
        out += varint(max(int(props.get("POP_EST") or 0), 0))
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


def main():
    src = Path(sys.argv[1] if len(sys.argv) > 1 else "data-src")
    dst = Path(sys.argv[2] if len(sys.argv) > 2 else "assets/data")
    dst.mkdir(parents=True, exist_ok=True)
    for f in sorted(src.glob("*.geojson")):
        if f.name == PLACES_SRC:
            continue
        gj = json.loads(f.read_text(encoding="utf-8"))
        data = encode_countries(gj) if "admin_0_countries" in f.stem else encode(gj)
        (dst / (f.stem + ".bin")).write_bytes(data)
        print(f"{f.name}: {f.stat().st_size} -> {len(data)} bytes")
    if (src / PLACES_SRC).exists() and (src / COUNTRIES_SRC).exists():
        data = build_places(src)
        (dst / "places.tsv").write_bytes(data)
        print(f"places.tsv: {len(data)} bytes")


if __name__ == "__main__":
    main()
