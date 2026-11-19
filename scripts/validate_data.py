#!/usr/bin/env python3
"""Rule-based validation of a built data directory (stdlib only, deterministic).

    validate_data.py DIR [--baseline DIR] [--allow-shrink NAME...]

DIR is a built data directory, e.g. dist/data after `trunk build --release`:
manifest.json plus files/<stem>.<hash><ext> (see scripts/hash_data.py). This is
the gate for data that is not byte-reproducible from committed sources (see
maps-8r8.1); run it before deploying or uploading data.

Checks, one line per problem on stderr, exit status 1 if there are any:

  manifest  parses, supported version, every listed file exists, each hashed
            name matches the content, no unlisted files, no absolute or `..`
            paths.
  size      every file within MAX_FILE_BYTES (Cloudflare Pages), the total
            within MAX_TOTAL_BYTES, and the per-file budgets in FILE_BUDGETS.
  format    each file decodes by its extension: .bin (MAPD or MAPC, the formats
            in src/geo.rs and src/country.rs: supported version, no trailing
            bytes, lon/lat in range, at least one feature), .tsv (UTF-8,
            header line, column count, lat/lon in range, at least one row) and
            .webp (RIFF header, dimensions) and tilesets.json (the tile pack
            list, see check_tilesets). Unknown extensions fail, so a new data
            type must add a rule here.
  shrink    with --baseline (another built data dir, such as the published
            one), a file (by manifest name) may not disappear or lose more
            than half of its bytes or features unless named in --allow-shrink.

The decoders mirror src/geo.rs `decode_shapes` and src/country.rs
`decode_countries`; keep them in step when the formats change.
"""

import argparse
import hashlib
import json
import struct
import sys
from pathlib import Path

MANIFEST = "manifest.json"
FILES_DIR = "files"
MANIFEST_VERSION = 1  # src/data.rs MANIFEST_VERSION, scripts/hash_data.py VERSION
HASH_LEN = 16  # scripts/hash_data.py

MAX_FILE_BYTES = 25 * 1024 * 1024  # Cloudflare Pages per-file limit
MAX_TOTAL_BYTES = 64 * 1024 * 1024  # whole data directory; today about 4 MB
FILE_BUDGETS = {  # logical name -> bytes
    "blue_marble.webp": 2 * 1024 * 1024,
    "terrain.webp": 2 * 1024 * 1024,
    "night_lights.webp": 2 * 1024 * 1024,
}
SHRINK_LIMIT = 0.5  # a file may keep at least this fraction of bytes and features

MAPD_VERSIONS = {1}
MAPC_VERSIONS = {3}
MAPD_SCALE = 10_000
MAX_COUNTRIES = 255  # country IDs are one byte in the raster
PALETTE_SIZE = 6
# name -> (header line, column count, lat column, lon column)
TSV_SCHEMAS = {
    "places.tsv": ("MAPSPLACES1", 8, 3, 4),
    "labels.tsv": ("MAPSLABELS1", 7, 2, 3),
}
# name -> (width, height) the renderer expects
WEBP_SIZES = {
    "blue_marble.webp": (4096, 2048),
    "terrain.webp": (4096, 2048),
    "night_lights.webp": (4096, 2048),
}


class Bad(ValueError):
    """A file failed a rule; the message says which."""


# ---- binary layers ----


class Reader:
    def __init__(self, data, pos=0):
        self.data, self.pos = data, pos

    def varint(self):
        value = 0
        for shift in range(0, 64, 7):
            if self.pos >= len(self.data):
                raise Bad("unexpected end of data")
            byte = self.data[self.pos]
            self.pos += 1
            value |= (byte & 0x7F) << shift
            if byte & 0x80 == 0:
                if shift == 63 and byte > 1:
                    break  # the tenth byte may only carry the top bit of a u64
                return value
        raise Bad("varint too long")

    def delta(self):
        z = self.varint()
        return (z >> 1) ^ -(z & 1)

    def byte(self, what):
        if self.pos >= len(self.data):
            raise Bad(f"truncated {what}")
        self.pos += 1
        return self.data[self.pos - 1]

    def string(self):
        n = self.varint()
        if n > len(self.data) - self.pos:
            raise Bad("string exceeds data size")
        raw = self.data[self.pos : self.pos + n]
        self.pos += n
        try:
            raw.decode("utf-8")
        except UnicodeDecodeError:
            raise Bad("invalid UTF-8") from None

    def point(self, x, y, max_x, max_y):
        x += self.delta()
        y += self.delta()
        if abs(x) > max_x or abs(y) > max_y:
            raise Bad("coordinate out of lon/lat range")
        return x, y

    def finish(self, what):
        if self.pos != len(self.data):
            raise Bad(f"trailing bytes after last {what}")


def _header(data, magic, versions, what):
    if len(data) < 5 or data[:4] != magic:
        raise Bad(f"not a {what} file")
    if data[4] not in versions:
        raise Bad(f"unsupported {what} version {data[4]}")


def decode_shapes(data):
    """MAPD layer (src/geo.rs). Returns the shape count."""
    _header(data, b"MAPD", MAPD_VERSIONS, "map data")
    r = Reader(data, 5)
    count = r.varint()
    if count > len(data) // 5:
        raise Bad("shape count exceeds data size")
    x = y = 0
    max_x, max_y = 180 * MAPD_SCALE, 90 * MAPD_SCALE
    for _ in range(count):
        n = r.varint() >> 1
        if n < 2 or n > (len(data) - r.pos) // 2:
            raise Bad("invalid point count")
        for _ in range(n):
            x, y = r.point(x, y, max_x, max_y)
    r.finish("shape")
    return count


def decode_countries(data):
    """MAPC layer (src/country.rs). Returns the country count."""
    _header(data, b"MAPC", MAPC_VERSIONS, "country data")
    r = Reader(data, 5)
    scale = r.varint()
    if scale == 0 or scale > 1_000_000:
        raise Bad("invalid coordinate scale")
    max_x, max_y = 180 * scale, 90 * scale
    count = r.varint()
    if count > MAX_COUNTRIES:
        raise Bad("too many countries")
    x = y = 0
    for _ in range(count):
        for _ in range(5):  # name, iso2, iso3, continent, subregion
            r.string()
        r.varint()  # population
        r.varint()  # gdp_md
        if r.byte("income group") > 5:
            raise Bad("income group out of range")
        for _ in range(2):  # pop_year, gdp_year
            if r.varint() > 0xFFFF:
                raise Bad("year too large")
        if r.byte("color") >= PALETTE_SIZE:
            raise Bad("color index out of range")
        n_polys = r.varint()
        if n_polys > (len(data) - r.pos) // 8:
            raise Bad("polygon count exceeds data size")
        for _ in range(n_polys):
            n_rings = r.varint()
            if n_rings == 0 or n_rings > (len(data) - r.pos) // 7:
                raise Bad("invalid ring count")
            for _ in range(n_rings):
                n = r.varint()
                if n < 3 or n > (len(data) - r.pos) // 2:
                    raise Bad("invalid point count")
                for _ in range(n):
                    x, y = r.point(x, y, max_x, max_y)
    r.finish("country")
    return count


def decode_bin(data):
    if data[:4] == b"MAPC":
        return decode_countries(data)
    return decode_shapes(data)


# ---- text and image ----


def check_tsv(name, data):
    """Returns the row count (header excluded)."""
    schema = TSV_SCHEMAS.get(name)
    if schema is None:
        raise Bad("no TSV schema for this file; add one to TSV_SCHEMAS")
    magic, columns, lat_col, lon_col = schema
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        raise Bad("invalid UTF-8") from None
    if not text.endswith("\n"):
        raise Bad("missing final newline")
    lines = text[:-1].split("\n")
    if lines[0] != magic:
        raise Bad(f"header is not {magic}")
    for number, line in enumerate(lines[1:], start=2):
        cells = line.split("\t")
        if len(cells) != columns:
            raise Bad(f"line {number}: {len(cells)} columns, expected {columns}")
        for col, limit in ((lat_col, 90.0), (lon_col, 180.0)):
            try:
                v = float(cells[col])
            except ValueError:
                raise Bad(f"line {number}: column {col + 1} is not a number") from None
            if not -limit <= v <= limit:  # also false for NaN
                raise Bad(f"line {number}: column {col + 1} out of range")
    return len(lines) - 1


def webp_size(data):
    if len(data) < 30 or data[:4] != b"RIFF" or data[8:12] != b"WEBP":
        raise Bad("not a WebP file")
    if struct.unpack("<I", data[4:8])[0] != len(data) - 8:
        raise Bad("RIFF size does not match the file size")
    kind = data[12:16]
    if kind == b"VP8 ":  # lossy: 14-bit dimensions after the 3-byte start code
        if data[23:26] != b"\x9d\x01\x2a":
            raise Bad("bad VP8 start code")
        w, h = struct.unpack("<HH", data[26:30])
        return w & 0x3FFF, h & 0x3FFF
    if kind == b"VP8L":
        if data[20] != 0x2F:
            raise Bad("bad VP8L signature")
        bits = struct.unpack("<I", data[21:25])[0]
        return (bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1
    if kind == b"VP8X":
        return (
            int.from_bytes(data[24:27], "little") + 1,
            int.from_bytes(data[27:30], "little") + 1,
        )
    raise Bad(f"unknown WebP chunk {kind!r}")


def check_webp(name, data):
    w, h = webp_size(data)
    if w == 0 or h == 0:
        raise Bad("zero image size")
    want = WEBP_SIZES.get(name)
    if want and (w, h) != want:
        raise Bad(f"image is {w}x{h}, expected {want[0]}x{want[1]}")
    return 1


# Layers that may have a tile pack (src/view.rs Layer::id of the image layers).
TILESET_LAYERS = {"imagery", "night", "terrain"}
TILESET_KEYS = {"layer", "url", "sha256", "size", "min_level", "max_level", "tile_size", "format"}
TILE_SIZE = 256
MAX_TILE_LEVEL = 10  # scripts/tile_pack.py MAX_LEVEL


def check_tilesets(data):
    """tilesets.json: the tile packs the app streams (docs/tile-streaming.md).
    An empty list is fine, so it returns None (no feature count): dropping
    tilesets still trips the byte shrink guard."""
    try:
        doc = json.loads(data.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        raise Bad("not valid JSON") from None
    if not isinstance(doc, dict) or set(doc) != {"version", "tilesets"}:
        raise Bad("want an object with exactly 'version' and 'tilesets'")
    if doc["version"] != 1:
        raise Bad(f"unsupported version {doc['version']!r}")
    if not isinstance(doc["tilesets"], list):
        raise Bad("'tilesets' is not a list")
    seen = set()
    for i, t in enumerate(doc["tilesets"]):
        where = f"tileset {i}"
        if not isinstance(t, dict) or set(t) != TILESET_KEYS:
            raise Bad(f"{where}: want exactly the keys {sorted(TILESET_KEYS)}")
        if t["layer"] not in TILESET_LAYERS:
            raise Bad(f"{where}: unknown layer {t['layer']!r}")
        if t["layer"] in seen:
            raise Bad(f"{where}: second tileset for {t['layer']!r}")
        seen.add(t["layer"])
        sha = t["sha256"]
        if not (isinstance(sha, str) and len(sha) == 64 and all(c in "0123456789abcdef" for c in sha)):
            raise Bad(f"{where}: sha256 is not 64 lowercase hex digits")
        url = t["url"]
        if not (isinstance(url, str) and url.startswith("https://") and url.endswith(".tiles")):
            raise Bad(f"{where}: url must be https and end in .tiles")
        if f".{sha[:HASH_LEN]}.tiles" not in url.rsplit("/", 1)[-1]:
            raise Bad(f"{where}: the url's file name does not carry the sha256 prefix")
        lo, hi = t["min_level"], t["max_level"]
        ints = (lo, hi, t["size"], t["tile_size"])
        if not all(isinstance(v, int) and not isinstance(v, bool) for v in ints):
            raise Bad(f"{where}: levels, size and tile_size must be integers")
        if not 0 <= lo <= hi <= MAX_TILE_LEVEL:
            raise Bad(f"{where}: bad levels {lo}..{hi}")
        if t["tile_size"] != TILE_SIZE or t["format"] != "webp":
            raise Bad(f"{where}: want {TILE_SIZE}px WebP tiles")
        directory = 16 + 8 * sum(2 ** (2 * z + 1) for z in range(lo, hi + 1))
        if t["size"] <= directory:
            raise Bad(f"{where}: size {t['size']} is not more than the directory ({directory} bytes)")
    return None


def check_format(name, data):
    """Features in a file, or raise Bad."""
    ext = Path(name).suffix
    if ext == ".bin":
        return decode_bin(data)
    if ext == ".tsv":
        return check_tsv(name, data)
    if ext == ".webp":
        return check_webp(name, data)
    if name == "tilesets.json":
        return check_tilesets(data)
    raise Bad(f"unknown file type {ext!r}; add a rule to validate_data.py")


# ---- manifest ----


def hashed_path(name, data):
    p = Path(name)
    digest = hashlib.sha256(data).hexdigest()[:HASH_LEN]
    return f"{FILES_DIR}/{p.stem}.{digest}{p.suffix}"


def read_manifest(root):
    """Returns (files dict, problems)."""
    path = root / MANIFEST
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        return {}, [f"{MANIFEST}: cannot read: {e}"]
    if not isinstance(manifest, dict) or manifest.get("version") != MANIFEST_VERSION:
        version = manifest.get("version") if isinstance(manifest, dict) else None
        return {}, [f"{MANIFEST}: unsupported version {version!r}"]
    files = manifest.get("files")
    if not isinstance(files, dict) or not files:
        return {}, [f"{MANIFEST}: no files"]
    if not all(isinstance(k, str) and isinstance(v, str) for k, v in files.items()):
        return {}, [f"{MANIFEST}: files must map names to paths"]
    return files, []


def unsafe(path):
    parts = path.replace("\\", "/").split("/")
    return path.startswith(("/", "\\")) or ".." in parts or (len(path) > 1 and path[1] == ":")


def load(root):
    """Check the manifest and files. Returns (problems, info) where info maps
    a logical name to (bytes, features or None)."""
    root = Path(root)
    files, problems = read_manifest(root)
    info = {}
    if problems:
        return problems, info
    listed = set()
    total = 0
    for name in sorted(files):
        rel = files[name]
        if unsafe(name) or "/" in name:
            problems.append(f"{name}: unsafe name in manifest")
            continue
        if unsafe(rel):
            problems.append(f"{name}: unsafe path {rel!r} in manifest")
            continue
        listed.add(rel)
        path = root / rel
        if not path.is_file():
            problems.append(f"{name}: listed file {rel} is missing")
            continue
        data = path.read_bytes()
        total += len(data)
        if rel != hashed_path(name, data):
            problems.append(f"{name}: {rel} does not match its content hash")
        if len(data) > MAX_FILE_BYTES:
            problems.append(f"{name}: {len(data)} bytes, over the {MAX_FILE_BYTES} byte file limit")
        budget = FILE_BUDGETS.get(name)
        if budget is not None and len(data) > budget:
            problems.append(f"{name}: {len(data)} bytes, over its {budget} byte budget")
        try:
            features = check_format(name, data)
            if features == 0:
                raise Bad("no features")
        except Bad as e:
            problems.append(f"{name}: {e}")
            features = None
        info[name] = (len(data), features)
    if total > MAX_TOTAL_BYTES:
        problems.append(f"total {total} bytes, over the {MAX_TOTAL_BYTES} byte limit")
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(root).as_posix()
        if rel != MANIFEST and rel not in listed:
            problems.append(f"{rel}: not listed in {MANIFEST}")
    return problems, info


def shrink_problems(info, base, allow):
    problems = []
    for name in sorted(base):
        if name in allow:
            continue
        if name not in info:
            problems.append(f"{name}: in the baseline but missing")
            continue
        (size, feats), (bsize, bfeats) = info[name], base[name]
        if size < bsize * SHRINK_LIMIT:
            problems.append(f"{name}: {size} bytes, less than half of the baseline's {bsize}")
        if feats is not None and bfeats is not None and feats < bfeats * SHRINK_LIMIT:
            problems.append(f"{name}: {feats} features, less than half of the baseline's {bfeats}")
    return problems


def validate(root, baseline=None, allow_shrink=()):
    problems, info = load(root)
    if baseline is not None:
        # The baseline is whatever is published now; only its sizes and
        # feature counts matter, so its own problems are not reported.
        _, base = load(baseline)
        if not base:
            problems.append(f"baseline {baseline}: no readable files")
        problems += shrink_problems(info, base, set(allow_shrink))
    return problems


def main(argv=None):
    ap = argparse.ArgumentParser(description="Validate a built data directory.")
    ap.add_argument("dir", help="built data directory (contains manifest.json)")
    ap.add_argument("--baseline", help="another built data directory to compare against")
    ap.add_argument(
        "--allow-shrink",
        nargs="+",
        default=[],
        metavar="NAME",
        help="manifest names allowed to shrink or disappear",
    )
    args = ap.parse_args(argv)
    problems = validate(args.dir, args.baseline, args.allow_shrink)
    for p in problems:
        print(f"validate_data: {p}", file=sys.stderr)
    if problems:
        return 1
    print(f"validate_data: {args.dir} ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
