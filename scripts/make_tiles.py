#!/usr/bin/env python3
"""Build the tile packs that sharpen the image layers at close zoom.

See docs/tile-streaming.md for the pyramid, the pack format (scripts/tile_pack.py)
and how packs are published. Each tileset is cut from the full-resolution
pinned source of its layer:

  imagery  Blue Marble NG 2004-07, 500 m: 8 tiles of 21600x21600
           (nasa-blue-marble-200407-500m-a1..d2)                                 levels 3-7
  night    Black Marble 2016, 500 m: 8 tiles of 21600x21600
           (nasa-black-marble-2016-500m-a1..d2)                                  levels 3-7
  terrain  Natural Earth hypso + ocean bottom, 10800x5400 (make_terrain.py)      levels 3-4

The 500 m tiles are checked against the pinned world images the bases are
built from (placement and color), then cut one at a time.

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


def pinned_image(name, size):
    from PIL import Image

    Image.MAX_IMAGE_PIXELS = None  # large sources, from a pinned hash
    try:
        src = data_sources.fetch(name)
    except (data_sources.DataSourceError, OSError) as e:
        sys.exit(f"error: {e}")
    im = Image.open(src)
    im = im if im.mode == "RGB" else im.convert("RGB")
    im.load()
    if im.size != size:
        sys.exit(f"{name}: unexpected size {im.size}, wanted {size[0]}x{size[1]}")
    return im


class Source:
    """A full-resolution layer image, split into cols x rows equal source
    tiles that each cover 360/cols by 180/rows degrees (column 0 from
    longitude -180, row 0 at the north). `load(c, r)` returns one tile;
    `world()` optionally returns a whole-world image at any size to check
    the tiles' placement against."""

    def __init__(self, cols, rows, load, world=None):
        self.cols, self.rows, self.load, self.world = cols, rows, load, world


def marble(prefix, world_name, world_size):
    """NASA's 500 m mosaics: eight 21600x21600 tiles named A1..D2, letters
    west to east from -180, 1 north and 2 south of the equator."""
    def load(c, r):
        return pinned_image(f"{prefix}-{'abcd'[c]}{r + 1}", (21600, 21600))

    return Source(4, 2, load, lambda: pinned_image(world_name, world_size))


def whole(make):
    return Source(1, 1, lambda c, r: make())


def terrain_image():
    import make_terrain

    return make_terrain.full_image()


# Layer id (src/view.rs Layer::id) -> (source, max level). The max level is
# the deepest that upsamples the source by at most 1.25x: 86400 texels around
# the world give level 7 (65536), whose next level would upsample 1.5x.
TILESETS = {
    "imagery": (lambda: marble("nasa-blue-marble-200407-500m",
                               "nasa-blue-marble-200407-21600", (21600, 10800)), 7),
    "night": (lambda: marble("nasa-black-marble-2016-500m",
                             "nasa-black-marble-2016", (13500, 6750)), 7),
    "terrain": (lambda: whole(terrain_image), 4),
}

# Source pixels added around each source tile from its neighbors before
# cutting: enough for the border texel plus the Lanczos filter (3 texels
# each side) at MIN_LEVEL, so tiles cut from one source tile match those
# cut from the whole world resampled at once.
MARGIN = 128
LANCZOS_SUPPORT = 3
# Mean difference (0-255) allowed between a source tile and its place in the
# world image; a misplaced tile differs by tens.
PLACEMENT_TOLERANCE = 4


def tile_box(z, x, y):
    """(south, west, north, east) degrees of tile (z, x, y)."""
    cols, rows = tile_pack.grid(z)
    west = -180 + 360 * x / cols
    north = 90 - 180 * y / rows
    return north - 180 / rows, west, north, west + 360 / cols


def source_box(src, c, r):
    """(south, west, north, east) degrees of source tile (c, r)."""
    west = -180 + 360 * c / src.cols
    north = 90 - 180 * r / src.rows
    return north - 180 / src.rows, west, north, west + 360 / src.cols


def touches(box, region):
    s, w, n, e = box
    rs, rw, rn, re_ = region
    return s < rn and n > rs and w < re_ and e > rw


def encode(im):
    buf = io.BytesIO()
    im.save(buf, "WEBP", quality=QUALITY, method=4)
    return buf.getvalue()


def strips(im, m):
    """The m-pixel edges of a source tile: left, right, top, bottom."""
    w, h = im.size
    return {"left": im.crop((0, 0, m, h)), "right": im.crop((w - m, 0, w, h)),
            "top": im.crop((0, 0, w, m)), "bottom": im.crop((0, h - m, w, h))}


def padded(src, c, r, im, edges, m):
    """Source tile (c, r) with m pixels of its neighbors around it, wrapping
    east-west; beyond the poles the edge row is repeated. `edges[(c, r)]`
    holds each tile's strips()."""
    from PIL import Image

    w, h = im.size
    out = Image.new(im.mode, (w + 2 * m, h + 2 * m))
    xs = {-1: (w - m, w, 0), 0: (0, w, m), 1: (0, m, m + w)}  # crop x0, x1, paste x
    for dr in (-1, 0, 1):
        rr = r + dr
        if not 0 <= rr < src.rows:
            continue
        for dc in (-1, 0, 1):
            cc = (c + dc) % src.cols
            x0, x1, px = xs[dc]
            if dr == 0:
                piece = im if dc == 0 else edges[(cc, rr)]["right" if dc < 0 else "left"]
                if dc != 0:
                    x0, x1 = 0, m
                py = m
            else:
                piece = edges[(cc, rr)]["bottom" if dr < 0 else "top"]
                py = 0 if dr < 0 else m + h
            out.paste(piece.crop((x0, 0, x1, piece.size[1])), (px, py))
    if r == 0:
        out.paste(out.crop((0, m, w + 2 * m, m + 1)).resize((w + 2 * m, m)), (0, 0))
    if r == src.rows - 1:
        out.paste(out.crop((0, m + h - 1, w + 2 * m, m + h)).resize((w + 2 * m, m)), (0, m + h))
    return out


def check_placement(src, c, r, im, world):
    from PIL import Image, ImageChops, ImageStat

    W, H = world.size
    s, w, n, e = source_box(src, c, r)
    box = ((w + 180) / 360 * W, (90 - n) / 180 * H, (e + 180) / 360 * W, (90 - s) / 180 * H)
    want = world.resize((256, 256), Image.BOX, box=box)
    diff = ImageStat.Stat(ImageChops.difference(im.resize((256, 256), Image.BOX), want)).mean
    if max(diff) > PLACEMENT_TOLERANCE:
        sys.exit(f"source tile ({c}, {r}) does not match its place in the world image "
                 f"(mean difference {max(diff):.1f})")


def cut(src, max_level, regions=None):
    """{(z, x, y): WebP bytes} for levels MIN_LEVEL..max_level, only the tiles
    touching `regions` if given. Each image is TILE_SIZE + 2 * BORDER square.
    Works one source tile at a time, so memory holds one source tile (plus
    neighbors' edges), never the world at the finest level."""
    from PIL import Image

    t = tile_pack.TILE_SIZE
    b = tile_pack.BORDER
    m = MARGIN
    cells = [(c, r) for r in range(src.rows) for c in range(src.cols)]
    if src.cols * src.rows == 1:
        only = src.load(0, 0)
        load = lambda c, r: only  # noqa: E731
    else:
        load = src.load
    world = src.world() if src.world else None
    edges = {}
    for c, r in cells:
        im = load(c, r)
        if world is not None:
            check_placement(src, c, r, im, world)
        edges[(c, r)] = strips(im, m)
        del im
    del world
    tiles = {}
    for c, r in cells:
        if regions is not None and not any(touches(source_box(src, c, r), g) for g in regions):
            continue
        im = load(c, r)
        w, h = im.size
        pad = padded(src, c, r, im, edges, m)
        del im
        for z in range(max_level, MIN_LEVEL - 1, -1):
            cols, rows = tile_pack.grid(z)
            if cols % src.cols or rows % src.rows:
                sys.exit(f"level {z} does not split evenly into {src.cols}x{src.rows} source tiles")
            tc, tr = cols // src.cols, rows // src.rows
            sx, sy = w / (tc * t), h / (tr * t)
            if (b + LANCZOS_SUPPORT) * max(sx, sy) > m:
                sys.exit(f"MARGIN {m} is too small for level {z}")
            wanted = [(lx, ly) for ly in range(tr) for lx in range(tc)
                      if regions is None
                      or any(touches(tile_box(z, c * tc + lx, r * tr + ly), g) for g in regions)]

            def one(p):
                lx, ly = p
                box = (m + (lx * t - b) * sx, m + (ly * t - b) * sy,
                       m + ((lx + 1) * t + b) * sx, m + ((ly + 1) * t + b) * sy)
                return encode(pad.resize((t + 2 * b, t + 2 * b), Image.LANCZOS, box=box))

            with ThreadPoolExecutor() as pool:
                for (lx, ly), data in zip(wanted, pool.map(one, wanted)):
                    tiles[(z, c * tc + lx, r * tr + ly)] = data
        print(f"source tile ({c}, {r}): {len(tiles)} tiles so far", file=sys.stderr)
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
