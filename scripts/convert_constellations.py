#!/usr/bin/env python3
"""Build assets/data/constellations.bin, the sky view's constellation
figures and names.

Sources, both pinned in data-sources.lock.json: the figures (polylines of
Hipparcos numbers) and Latin names of Stellarium's `western` sky culture
(`stellarium-western`, CC BY-SA), and star positions from the HYG database
(`hyg-v44`, CC BY-SA 4.0). The output is a derived work offered under CC
BY-SA 4.0, kept in its own file. Stdlib only; the output is deterministic.

    python3 scripts/convert_constellations.py                     # pinned sources
    python3 scripts/convert_constellations.py index.json hyg.csv.gz

Format "CNST" version 1, little-endian (read by src/constellations.rs,
checked by scripts/validate_data.py). Positions are J2000, quantized as in
stars.bin (u32 right ascension as a fraction of a turn times 2^32, i32
declination as a fraction of 90 degrees times 2^31 - 1):

    0   4   magic b"CNST"
    4   1   version (1)
    5   3   reserved (0)
    8   4   constellation count C
    12  ... C constellations, sorted by abbreviation:
            3   IAU abbreviation, ASCII
            1   name byte length, then the Latin name in UTF-8
            8   label position (RA, Dec): the normalized mean of the
                figure's stars
            2   u16 polyline count, then each polyline:
                1   flags: 1 = a thin (secondary) line
                2   u16 point count (2 or more), then 8 bytes per point
"""
import csv
import gzip
import json
import math
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

FIGURES = "stellarium-western"
STARS = "hyg-v44"
MAGIC = b"CNST"
VERSION = 1
HEADER = struct.Struct("<4sB3xI")
POINT = struct.Struct("<Ii")
THIN = 1
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "data", "constellations.bin")


def quantize(ra_deg, dec_deg):
    ra = round((ra_deg / 360.0) % 1.0 * 2**32) % 2**32
    dec = round(max(-1.0, min(1.0, dec_deg / 90.0)) * (2**31 - 1))
    return ra, dec


def hip_positions(text_stream):
    """{hip: (ra_deg, dec_deg)} from a HYG CSV."""
    out = {}
    for r in csv.DictReader(text_stream):
        if r["hip"] and r["ra"] and r["dec"]:
            out[int(r["hip"])] = (float(r["ra"]) * 15.0, float(r["dec"]))
    return out


def unit(ra_deg, dec_deg):
    ra, dec = math.radians(ra_deg), math.radians(dec_deg)
    return (math.cos(dec) * math.sin(ra), math.sin(dec), math.cos(dec) * math.cos(ra))


def label_point(points):
    """RA and Dec (degrees) of the normalized mean of the stars' directions."""
    sx = sum(unit(*p)[0] for p in points)
    sy = sum(unit(*p)[1] for p in points)
    sz = sum(unit(*p)[2] for p in points)
    n = math.sqrt(sx * sx + sy * sy + sz * sz)
    if n < 1e-9:
        raise ValueError("figure stars cancel out")
    ra = math.degrees(math.atan2(sx, sz)) % 360.0
    # A tiny negative angle wraps to 360.0 in floating point.
    return (0.0 if ra >= 360.0 else ra), math.degrees(math.asin(sy / n))


def figures(index, positions):
    """[(abbr, name, label, [(flags, [(ra, dec)])])] sorted by abbreviation."""
    out = []
    for c in index["constellations"]:
        abbr = c["iau"]
        if len(abbr) != 3 or not abbr.isascii():
            raise ValueError(f"bad abbreviation {abbr!r}")
        name = c["common_name"]["native"]
        lines = []
        stars = {}
        for line in c["lines"]:
            flags = 0
            if line and line[0] == "thin":
                flags, line = THIN, line[1:]
            if len(line) < 2 or not all(isinstance(h, int) for h in line):
                raise ValueError(f"{abbr}: bad line {line!r}")
            missing = [h for h in line if h not in positions]
            if missing:
                raise ValueError(f"{abbr}: no position for HIP {missing}")
            lines.append((flags, [positions[h] for h in line]))
            stars.update((h, positions[h]) for h in line)
        if not lines:
            raise ValueError(f"{abbr}: no lines")
        label = label_point([stars[h] for h in sorted(stars)])
        out.append((abbr, name, label, lines))
    out.sort(key=lambda f: f[0])
    if len({f[0] for f in out}) != len(out):
        raise ValueError("duplicate abbreviations")
    return out


def encode(figs):
    out = bytearray(HEADER.pack(MAGIC, VERSION, len(figs)))
    for abbr, name, label, lines in figs:
        b = name.encode("utf-8")
        out += abbr.encode("ascii") + struct.pack("<B", len(b)) + b
        out += POINT.pack(*quantize(*label))
        out += struct.pack("<H", len(lines))
        for flags, points in lines:
            out += struct.pack("<BH", flags, len(points))
            for p in points:
                out += POINT.pack(*quantize(*p))
    return bytes(out)


def decode(data):
    """[(abbr, name, (ra_rad, dec_rad), [(flags, [(ra_rad, dec_rad)])])]."""
    def point(at):
        ra, dec = POINT.unpack_from(data, at)
        return ra / 2**32 * 2 * math.pi, dec / (2**31 - 1) * math.pi / 2

    magic, version, count = HEADER.unpack_from(data, 0)
    if magic != MAGIC or version != VERSION:
        raise ValueError("not a CNST v1 file")
    at = HEADER.size
    out = []
    try:
        for _ in range(count):
            abbr = data[at:at + 3].decode("ascii")
            length = data[at + 3]
            name = data[at + 4:at + 4 + length].decode("utf-8")
            at += 4 + length
            label = point(at)
            (n_lines,) = struct.unpack_from("<H", data, at + 8)
            at += 10
            lines = []
            for _ in range(n_lines):
                flags, n = struct.unpack_from("<BH", data, at)
                at += 3
                if n < 2 or flags & ~THIN:
                    raise ValueError(f"{abbr}: bad polyline")
                lines.append((flags, [point(at + 8 * i) for i in range(n)]))
                at += 8 * n
            if len(abbr) != 3 or not name or not lines:
                raise ValueError("bad constellation entry")
            out.append((abbr, name, label, lines))
    except (struct.error, IndexError, UnicodeDecodeError) as e:
        raise ValueError(f"truncated or malformed constellation data: {e}")
    if at != len(data):
        raise ValueError("trailing bytes after the constellations")
    return out


def main():
    if len(sys.argv) > 2:
        index_path, hyg_path = sys.argv[1], sys.argv[2]
    else:
        import data_sources

        index_path, hyg_path = data_sources.fetch(FIGURES), data_sources.fetch(STARS)
    with open(index_path, encoding="utf-8") as f:
        index = json.load(f)
    with gzip.open(hyg_path, "rt", encoding="utf-8", newline="") as f:
        positions = hip_positions(f)
    figs = figures(index, positions)
    data = encode(figs)
    with open(OUT, "wb") as f:
        f.write(data)
    lines = sum(len(f[3]) for f in figs)
    print(f"wrote {os.path.normpath(OUT)}: {len(figs)} constellations, {lines} polylines, {len(data)} bytes")


if __name__ == "__main__":
    main()
