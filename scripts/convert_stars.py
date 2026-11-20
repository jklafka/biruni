#!/usr/bin/env python3
"""Build assets/data/stars.bin, the sky view's star catalog, from HYG.

Source: the HYG database v4.4 by David Nash (astronexus), pinned in
data-sources.lock.json as `hyg-v44` (CC BY-SA 4.0). stars.bin is a derived
work and is offered under CC BY-SA 4.0 too; it is kept in its own file so the
license does not reach other data. Stdlib only; the output is deterministic.

    python3 scripts/convert_stars.py              # fetch the pinned file, then convert
    python3 scripts/convert_stars.py hyg.csv.gz   # convert a local copy

Format "STAR" version 1, little-endian (read by src/stars.rs, checked by
scripts/validate_data.py):

    0   4   magic b"STAR"
    4   1   version (1)
    5   3   reserved (0)
    8   4   star count N
    12  4   name count K
    16  10N stars, brightest first (ties by right ascension, then declination):
            u32 right ascension, J2000, as a fraction of a turn times 2^32
            i32 declination, J2000, as a fraction of 90 degrees times 2^31 - 1
            u8  visual magnitude: round((mag + 2) * 20), so -2.0 to 10.75 in 0.05 steps
            i8  B-V color index: round(ci * 50), -128 when unknown
    ...     K names, in star order: u32 star index, u8 byte length, UTF-8 bytes

The Sun (HYG id 0) is left out; the sky view places it from the ephemeris.
"""
import csv
import gzip
import io
import math
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

SOURCE = "hyg-v44"
MAG_LIMIT = 8.0
MAGIC = b"STAR"
VERSION = 1
HEADER = struct.Struct("<4sB3xII")
STAR = struct.Struct("<IiBb")
NO_CI = -128
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "data", "stars.bin")


def quantize(ra_hours, dec_deg, mag, ci):
    ra = round((ra_hours / 24.0) % 1.0 * 2**32) % 2**32
    dec = round(max(-1.0, min(1.0, dec_deg / 90.0)) * (2**31 - 1))
    m = max(0, min(255, round((mag + 2.0) * 20.0)))
    c = NO_CI if ci is None else max(-127, min(127, round(ci * 50.0)))
    return ra, dec, m, c


def read_rows(text_stream):
    """(ra_hours, dec_deg, mag, ci or None, proper name) for each usable star
    brighter than MAG_LIMIT, the Sun excluded."""
    out = []
    for r in csv.DictReader(text_stream):
        if r["id"] == "0" or not r["mag"] or not r["ra"] or not r["dec"]:
            continue
        mag = float(r["mag"])
        if mag > MAG_LIMIT:
            continue
        ra, dec = float(r["ra"]), float(r["dec"])
        if not all(math.isfinite(v) for v in (ra, dec, mag)):
            continue
        ci = float(r["ci"]) if r["ci"] else None
        out.append((ra, dec, mag, ci if ci is None or math.isfinite(ci) else None, r["proper"].strip()))
    return out


def encode(rows):
    stars = []
    for ra, dec, mag, ci, name in rows:
        stars.append((quantize(ra, dec, mag, ci), name))
    # Brightest first; ties broken by the quantized position so the order
    # never depends on the CSV's.
    stars.sort(key=lambda s: (s[0][2], s[0][0], s[0][1], s[1]))
    names = [(i, n.encode("utf-8")) for i, (_, n) in enumerate(stars) if n]
    out = io.BytesIO()
    out.write(HEADER.pack(MAGIC, VERSION, len(stars), len(names)))
    for q, _ in stars:
        out.write(STAR.pack(*q))
    for i, b in names:
        if len(b) > 255:
            raise ValueError(f"star name too long: {b!r}")
        out.write(struct.pack("<IB", i, len(b)))
        out.write(b)
    return out.getvalue()


def decode(data):
    """[(ra_rad, dec_rad, mag, ci or None, name or "")]: the inverse of
    encode, for tests and validate_data.py."""
    magic, version, n, k = HEADER.unpack_from(data, 0)
    if magic != MAGIC or version != VERSION:
        raise ValueError("not a STAR v1 file")
    if len(data) < HEADER.size + STAR.size * n:
        raise ValueError("truncated star records")
    stars = []
    for i in range(n):
        ra, dec, m, c = STAR.unpack_from(data, HEADER.size + STAR.size * i)
        stars.append([ra / 2**32 * 2 * math.pi, dec / (2**31 - 1) * math.pi / 2,
                      m / 20.0 - 2.0, None if c == NO_CI else c / 50.0, ""])
    at = HEADER.size + STAR.size * n
    for _ in range(k):
        if at + 5 > len(data):
            raise ValueError("truncated name table")
        i, length = struct.unpack_from("<IB", data, at)
        at += 5
        if i >= n or at + length > len(data):
            raise ValueError("bad name entry")
        stars[i][4] = data[at:at + length].decode("utf-8")
        at += length
    if at != len(data):
        raise ValueError("trailing bytes after the name table")
    return [tuple(s) for s in stars]


def main():
    if len(sys.argv) > 1:
        src = sys.argv[1]
    else:
        import data_sources

        src = data_sources.fetch(SOURCE)
    with gzip.open(src, "rt", encoding="utf-8", newline="") as f:
        rows = read_rows(f)
    data = encode(rows)
    with open(OUT, "wb") as f:
        f.write(data)
    print(f"wrote {os.path.normpath(OUT)}: {len(rows)} stars, {len(data)} bytes")


if __name__ == "__main__":
    main()
