#!/usr/bin/env python3
"""Tile packs: one file holding an equirectangular tile pyramid (stdlib only).

The format is described in docs/tile-streaming.md and read by src/tile_pack.rs;
keep the three in step. Little-endian:

    0   4   magic b"BTIL"
    4   1   version (1)
    5   1   min level
    6   1   max level
    7   1   tile format (1 = WebP)
    8   2   tile size (256): texels of the world each tile covers
    10  2   border (1): extra texels on each side, copied from the neighbors
            (wrapping in longitude, repeating the edge at the poles), so a
            stored tile image is size + 2 * border square
    12  4   tile count N, the sum of 2^(2z+1) over the levels
    16  8N  directory: u32 offset, u32 length per tile (length 0 = absent)
    ...     tile bytes

Level z has 2^(z+1) columns and 2^z rows. The directory and the tile bytes are
ordered by level, then row (north first), then column (from longitude -180).
"""

import struct

MAGIC = b"BTIL"
VERSION = 1
FORMAT_WEBP = 1
FORMATS = {FORMAT_WEBP: "webp"}
TILE_SIZE = 256
BORDER = 1
HEADER = struct.Struct("<4sBBBBHHI")
ENTRY = struct.Struct("<II")
MAX_LEVEL = 10  # 2^21 tiles; far beyond any source we have


class PackError(ValueError):
    pass


def grid(z):
    """(columns, rows) of level z."""
    return 2 ** (z + 1), 2**z


def tile_count(min_level, max_level):
    return sum(c * r for c, r in (grid(z) for z in range(min_level, max_level + 1)))


def header_len(min_level, max_level):
    """Bytes of the header plus directory, which the app reads in one request."""
    return HEADER.size + ENTRY.size * tile_count(min_level, max_level)


def index(min_level, z, x, y):
    """Directory position of tile (z, x, y)."""
    cols, rows = grid(z)
    if not (0 <= x < cols and 0 <= y < rows):
        raise PackError(f"tile ({z}, {x}, {y}) is outside level {z}")
    return tile_count(min_level, z - 1) + y * cols + x


def check_levels(min_level, max_level):
    if not (0 <= min_level <= max_level <= MAX_LEVEL):
        raise PackError(f"bad levels {min_level}..{max_level}")


def write(min_level, max_level, tiles, tile_size=TILE_SIZE, fmt=FORMAT_WEBP, border=BORDER):
    """Pack bytes for `tiles`, a dict {(z, x, y): encoded bytes}. Missing
    tiles get length 0."""
    check_levels(min_level, max_level)
    n = tile_count(min_level, max_level)
    entries, body = [], []
    offset = header_len(min_level, max_level)
    for z in range(min_level, max_level + 1):
        cols, rows = grid(z)
        for y in range(rows):
            for x in range(cols):
                data = tiles.get((z, x, y), b"")
                entries.append(ENTRY.pack(offset if data else 0, len(data)))
                body.append(data)
                offset += len(data)
    extra = set(tiles) - {(z, x, y) for z in range(min_level, max_level + 1)
                          for y in range(grid(z)[1]) for x in range(grid(z)[0])}
    if extra:
        raise PackError(f"tiles outside the pyramid: {sorted(extra)[:3]}")
    if offset >= 2**32:
        raise PackError("pack over 4 GiB")
    head = HEADER.pack(MAGIC, VERSION, min_level, max_level, fmt, tile_size, border, n)
    return head + b"".join(entries) + b"".join(body)


def read_header(data):
    """Parse the header and directory. Returns (info dict, [(offset, length)])."""
    if len(data) < HEADER.size:
        raise PackError("truncated header")
    magic, version, lo, hi, fmt, size, border, n = HEADER.unpack_from(data)
    if magic != MAGIC:
        raise PackError(f"bad magic {magic!r}")
    if version != VERSION:
        raise PackError(f"unsupported version {version}")
    check_levels(lo, hi)
    if fmt not in FORMATS:
        raise PackError(f"unknown tile format {fmt}")
    if size == 0:
        raise PackError("tile size 0")
    if n != tile_count(lo, hi):
        raise PackError(f"tile count {n} does not match levels {lo}..{hi}")
    end = header_len(lo, hi)
    if len(data) < end:
        raise PackError("truncated directory")
    entries = [ENTRY.unpack_from(data, HEADER.size + i * ENTRY.size) for i in range(n)]
    info = {"min_level": lo, "max_level": hi, "format": FORMATS[fmt], "tile_size": size,
            "border": border, "count": n}
    return info, entries


def check(data):
    """Parse a whole pack and check every entry lies inside it, after the
    directory, without overlaps. Returns the info dict plus the number of
    present tiles."""
    info, entries = read_header(data)
    start = header_len(info["min_level"], info["max_level"])
    spans = sorted((o, length) for o, length in entries if length)
    pos = start
    for o, length in spans:
        if o < pos:
            raise PackError(f"tile at {o} overlaps the directory or another tile")
        if o + length > len(data):
            raise PackError(f"tile at {o} runs past the end of the pack")
        pos = o + length
    for o, length in entries:
        if length == 0 and o != 0:
            raise PackError("absent tile with a nonzero offset")
    return dict(info, present=len(spans))


def tile(data, min_level, z, x, y):
    """Encoded bytes of tile (z, x, y), or None if absent."""
    o, length = ENTRY.unpack_from(data, HEADER.size + ENTRY.size * index(min_level, z, x, y))
    return data[o:o + length] if length else None
