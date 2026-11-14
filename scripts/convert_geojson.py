#!/usr/bin/env python3
"""Convert Natural Earth GeoJSON into the compact `.bin` format read by
`geo::decode_shapes` (see the format description in src/geo.rs).

Usage: python3 scripts/convert_geojson.py [data-src [assets/data]]

Mirrors `geo::parse_shapes`: points are skipped, positions with fewer than two
coordinates are dropped, shapes with fewer than two points are discarded.
`ne_50m_admin_0_countries.geojson` is converted to the separate country format
(`MAPC`, see src/country.rs): polygons with a small attribute table.
Output is deterministic, so rerunning it must not change the committed files.
"""
import json
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


def main():
    src = Path(sys.argv[1] if len(sys.argv) > 1 else "data-src")
    dst = Path(sys.argv[2] if len(sys.argv) > 2 else "assets/data")
    dst.mkdir(parents=True, exist_ok=True)
    for f in sorted(src.glob("*.geojson")):
        gj = json.loads(f.read_text(encoding="utf-8"))
        data = encode_countries(gj) if "admin_0_countries" in f.stem else encode(gj)
        (dst / (f.stem + ".bin")).write_bytes(data)
        print(f"{f.name}: {f.stat().st_size} -> {len(data)} bytes")


if __name__ == "__main__":
    main()
