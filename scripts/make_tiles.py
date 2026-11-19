#!/usr/bin/env python3
"""Build the tile packs that sharpen the image layers at close zoom.

See docs/tile-streaming.md for the pyramid, the pack format (scripts/tile_pack.py)
and how packs are published. Each tileset is cut from the full-resolution
pinned source of its layer:

  imagery  Blue Marble NG 2004-07, 21600x10800 (nasa-blue-marble-200407-21600)  levels 3-5
  night    Black Marble 2016, 13500x6750 (scripts/make_night_lights.py)          levels 3-5
  terrain  Natural Earth hypso + ocean bottom, 10800x5400 (make_terrain.py)      levels 3-4

    python3 scripts/make_tiles.py build imagery [--tilesets PATH]
        Write .data-cache/tiles/imagery.<sha16>.tiles and point the layer's
        entry in assets/data/tilesets.json at its R2 URL. Upload the pack to
        R2 before merging that change (the publish-tiles workflow does both).

    python3 scripts/make_tiles.py fixture imagery OUT --box S,W,N,E [--box ...]
        Write a sparse pack with only the tiles that touch the boxes (degrees),
        for tests.

Needs Pillow with WebP support (plus NumPy for terrain). WebP output is not
byte-for-byte reproducible across libwebp versions; a pack is identified by
the hash recorded when it was built.
"""
import argparse
import hashlib
import io
import json
import os
import sys
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import data_sources  # noqa: E402
import tile_pack  # noqa: E402

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
TILESETS_JSON = os.path.join(ROOT, "assets", "data", "tilesets.json")
CACHE = os.path.join(ROOT, ".data-cache", "tiles")
BASE_URL = "https://biruni-data.jklafka.cloud"  # deploy.yml R2_BASE_URL
MIN_LEVEL = 3
QUALITY = 80
TILESETS_VERSION = 1


def imagery_image():
    from PIL import Image

    Image.MAX_IMAGE_PIXELS = None  # 233 MP source, from a pinned hash
    try:
        src = data_sources.fetch("nasa-blue-marble-200407-21600")
    except (data_sources.DataSourceError, OSError) as e:
        sys.exit(f"error: {e}")
    im = Image.open(src).convert("RGB")
    if im.size != (21600, 10800):
        sys.exit(f"unexpected source size {im.size}, wanted 21600x10800")
    return im


def night_image():
    import make_night_lights

    return make_night_lights.full_image()


def terrain_image():
    import make_terrain

    return make_terrain.full_image()


# Layer id (src/view.rs Layer::id) -> (full-resolution image, max level).
# The max level is the deepest that upsamples the source by at most 1.25x.
TILESETS = {
    "imagery": (imagery_image, 5),
    "night": (night_image, 5),
    "terrain": (terrain_image, 4),
}


def level_size(z):
    cols, rows = tile_pack.grid(z)
    return cols * tile_pack.TILE_SIZE, rows * tile_pack.TILE_SIZE


def tile_box(z, x, y):
    """(south, west, north, east) degrees of tile (z, x, y)."""
    cols, rows = tile_pack.grid(z)
    west = -180 + 360 * x / cols
    north = 90 - 180 * y / rows
    return north - 180 / rows, west, north, west + 360 / cols


def touches(box, region):
    s, w, n, e = box
    rs, rw, rn, re_ = region
    return s < rn and n > rs and w < re_ and e > rw


def encode(im):
    buf = io.BytesIO()
    im.save(buf, "WEBP", quality=QUALITY, method=4)
    return buf.getvalue()


def padded(level, b):
    """The level image with a border of b texels: wrapped east-west, the
    edge rows repeated at the poles. Tiles cut from it overlap their
    neighbors by b, so bilinear filtering blends across tile edges."""
    from PIL import Image

    w, h = level.size
    out = Image.new(level.mode, (w + 2 * b, h + 2 * b))
    out.paste(level, (b, b))
    out.paste(level.crop((w - b, 0, w, h)), (0, b))
    out.paste(level.crop((0, 0, b, h)), (w + b, b))
    out.paste(out.crop((0, b, w + 2 * b, 2 * b)), (0, 0))
    out.paste(out.crop((0, h, w + 2 * b, h + b)), (0, h + b))
    return out


def cut(full, max_level, regions=None):
    """{(z, x, y): WebP bytes} for levels MIN_LEVEL..max_level, only the tiles
    touching `regions` if given. Each image is TILE_SIZE + 2 * BORDER square."""
    from PIL import Image

    tiles = {}
    t = tile_pack.TILE_SIZE
    b = tile_pack.BORDER
    for z in range(max_level, MIN_LEVEL - 1, -1):
        cols, rows = tile_pack.grid(z)
        wanted = [(x, y) for y in range(rows) for x in range(cols)
                  if regions is None or any(touches(tile_box(z, x, y), r) for r in regions)]
        if not wanted:
            continue
        level = padded(full.resize(level_size(z), Image.LANCZOS), b)
        with ThreadPoolExecutor() as pool:
            crops = [level.crop((x * t, y * t, (x + 1) * t + 2 * b, (y + 1) * t + 2 * b)) for x, y in wanted]
            for (x, y), data in zip(wanted, pool.map(encode, crops)):
                tiles[(z, x, y)] = data
        print(f"level {z}: {len(wanted)} tiles", file=sys.stderr)
    return tiles


def load_tilesets(path):
    if not os.path.exists(path):
        return {"version": TILESETS_VERSION, "tilesets": []}
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def tilesets_text(doc):
    doc["tilesets"].sort(key=lambda t: t["layer"])
    return json.dumps(doc, indent=2, sort_keys=True) + "\n"


def entry(layer, name, data, max_level):
    return {
        "layer": layer,
        "url": f"{BASE_URL}/files/{name}",
        "sha256": hashlib.sha256(data).hexdigest(),
        "size": len(data),
        "min_level": MIN_LEVEL,
        "max_level": max_level,
        "tile_size": tile_pack.TILE_SIZE,
        "format": "webp",
    }


def build(layer, tilesets_path):
    make, max_level = TILESETS[layer]
    data = tile_pack.write(MIN_LEVEL, max_level, cut(make(), max_level))
    tile_pack.check(data)
    name = f"{layer}.{hashlib.sha256(data).hexdigest()[:16]}.tiles"
    os.makedirs(CACHE, exist_ok=True)
    out = os.path.join(CACHE, name)
    with open(out, "wb") as f:
        f.write(data)
    doc = load_tilesets(tilesets_path)
    doc["tilesets"] = [t for t in doc["tilesets"] if t.get("layer") != layer]
    doc["tilesets"].append(entry(layer, name, data, max_level))
    with open(tilesets_path, "wb") as f:
        f.write(tilesets_text(doc).encode("utf-8"))
    print(f"wrote {out}: {len(data)} bytes; updated {tilesets_path}")
    return out


def fixture(layer, out, regions):
    make, max_level = TILESETS[layer]
    data = tile_pack.write(MIN_LEVEL, max_level, cut(make(), max_level, regions))
    info = tile_pack.check(data)
    with open(out, "wb") as f:
        f.write(data)
    print(f"wrote {out}: {len(data)} bytes, {info['present']} tiles")


def parse_box(s):
    try:
        south, west, north, east = (float(v) for v in s.split(","))
    except ValueError:
        raise argparse.ArgumentTypeError(f"bad box {s!r}, want S,W,N,E")
    return south, west, north, east


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build")
    b.add_argument("layer", choices=sorted(TILESETS))
    b.add_argument("--tilesets", default=TILESETS_JSON)
    f = sub.add_parser("fixture")
    f.add_argument("layer", choices=sorted(TILESETS))
    f.add_argument("out")
    f.add_argument("--box", type=parse_box, action="append", required=True)
    a = p.parse_args(argv)
    if a.cmd == "build":
        build(a.layer, a.tilesets)
    else:
        fixture(a.layer, a.out, a.box)


if __name__ == "__main__":
    main()
