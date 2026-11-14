#!/usr/bin/env python3
"""Convert Natural Earth GeoJSON into the compact `.bin` format read by
`geo::decode_shapes` (see the format description in src/geo.rs).

Usage: python3 scripts/convert_geojson.py [data-src [assets/data]]

Mirrors `geo::parse_shapes`: points are skipped, positions with fewer than two
coordinates are dropped, shapes with fewer than two points are discarded.
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


def main():
    src = Path(sys.argv[1] if len(sys.argv) > 1 else "data-src")
    dst = Path(sys.argv[2] if len(sys.argv) > 2 else "assets/data")
    dst.mkdir(parents=True, exist_ok=True)
    for f in sorted(src.glob("*.geojson")):
        data = encode(json.loads(f.read_text(encoding="utf-8")))
        (dst / (f.stem + ".bin")).write_bytes(data)
        print(f"{f.name}: {f.stat().st_size} -> {len(data)} bytes")


if __name__ == "__main__":
    main()
