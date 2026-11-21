#!/usr/bin/env python3
"""The "HIST" time-indexed vector format (docs/historical-data.md): features
that each hold for an interval of time, so any moment can be drawn without
per-year snapshots. Read by src/history.rs and checked by
scripts/validate_data.py; written by the converters of the historical layers.
Stdlib only; encoding is deterministic.

Little-endian, varints are unsigned LEB128, "zigzag" varints are signed:

    0   4   magic b"HIST"
    4   1   version (1)
    5   1   time scale: 0 = calendar years (astronomical numbering is NOT
            used: -1 is 1 BCE and there is no year 0, as in Cliopatria);
            1 = geological time, stored as -(age in tenths of a million
            years), so that later is always larger
    6   1   coordinate digits D: lon/lat are integers in units of 10^-D degree
    7   1   reserved (0)
    8   ... varint name count, then each name: varint byte length, UTF-8
        ... varint ring count, then each ring: varint point count N (>= 3),
            then N pairs of zigzag deltas (lon, lat) from the previous point
            of the file (the first from 0,0); rings are closed implicitly
        ... varint geometry count, then each geometry: varint ring count K,
            then K varint ring indices; a geometry's rings are filled with
            the even-odd rule. Rings are shared: a polity whose borders
            change in one place keeps its other rings
        ... varint record count, then each record:
            varint name index, zigzag from, zigzag to (inclusive: the
            record holds when from <= t <= to), varint geometry index,
            zigzag label lon, zigzag label lat (absolute, where the name
            goes), varint tag (layer-specific: the fill color and label
            tier for borders, the plate id for geology; 0 when unused)
        end of file (no trailing bytes)
"""
import struct

MAGIC = b"HIST"
VERSION = 1
SCALE_YEARS = 0
SCALE_MA = 1
HEADER = struct.Struct("<4sBBBx")


def put_varint(out, n):
    if n < 0:
        raise ValueError("negative varint")
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return


def zigzag(n):
    return (n << 1) if n >= 0 else ((-n) << 1) - 1


class Reader:
    def __init__(self, data, pos=0):
        self.data, self.pos = data, pos

    def varint(self):
        value, shift = 0, 0
        while True:
            if self.pos >= len(self.data):
                raise ValueError("truncated")
            b = self.data[self.pos]
            self.pos += 1
            value |= (b & 0x7F) << shift
            if not b & 0x80:
                return value
            shift += 7
            if shift > 63:
                raise ValueError("varint too long")

    def zigzag(self):
        z = self.varint()
        return (z >> 1) ^ -(z & 1)

    def take(self, n):
        if self.pos + n > len(self.data):
            raise ValueError("truncated")
        chunk = self.data[self.pos:self.pos + n]
        self.pos += n
        return chunk


def encode(scale, digits, names, geometries, records):
    """names: [str]; geometries: [[ring]] with ring = [(lon, lat)] in
    degrees; records: [(name index, from, to, geometry index, (label lon,
    label lat), tag)]. Coordinates are rounded to `digits` decimals; rings
    that are equal after rounding are stored once."""
    k = 10 ** digits
    out = bytearray(HEADER.pack(MAGIC, VERSION, scale, digits))
    put_varint(out, len(names))
    for n in names:
        raw = n.encode("utf-8")
        put_varint(out, len(raw))
        out += raw
    ring_index, ring_list, geom_refs = {}, [], []
    for rings in geometries:
        refs = []
        for ring in rings:
            if len(ring) < 3:
                raise ValueError("ring with fewer than 3 points")
            q = tuple((round(lon * k), round(lat * k)) for lon, lat in ring)
            if q not in ring_index:
                ring_index[q] = len(ring_list)
                ring_list.append(q)
            refs.append(ring_index[q])
        geom_refs.append(refs)
    put_varint(out, len(ring_list))
    x = y = 0
    for ring in ring_list:
        put_varint(out, len(ring))
        for qx, qy in ring:
            out += bytes(_zz(qx - x)) + bytes(_zz(qy - y))
            x, y = qx, qy
    put_varint(out, len(geom_refs))
    for refs in geom_refs:
        put_varint(out, len(refs))
        for r in refs:
            put_varint(out, r)
    put_varint(out, len(records))
    for name, start, end, geom, (llon, llat), tag in records:
        if not (0 <= name < len(names) and 0 <= geom < len(geometries) and start <= end):
            raise ValueError(f"bad record {name, start, end, geom}")
        put_varint(out, name)
        put_varint(out, zigzag(start))
        put_varint(out, zigzag(end))
        put_varint(out, geom)
        put_varint(out, zigzag(round(llon * k)))
        put_varint(out, zigzag(round(llat * k)))
        put_varint(out, tag)
    return bytes(out)


def _zz(n):
    out = bytearray()
    put_varint(out, zigzag(n))
    return out


def decode(data):
    """(scale, digits, names, geometries, records) as `encode` takes them
    (coordinates as quantized degrees). Raises ValueError on bad input."""
    if len(data) < HEADER.size:
        raise ValueError("too short")
    magic, version, scale, digits = HEADER.unpack_from(data, 0)
    if magic != MAGIC or version != VERSION:
        raise ValueError(f"not HIST v1: {magic!r} {version}")
    if scale not in (SCALE_YEARS, SCALE_MA) or not 0 <= digits <= 6:
        raise ValueError(f"bad scale {scale} or digits {digits}")
    k = 10 ** digits
    r = Reader(data, HEADER.size)
    names = []
    for _ in range(_count(r, data)):
        names.append(r.take(r.varint()).decode("utf-8"))
    ring_list = []
    x = y = 0
    for _ in range(_count(r, data)):
        n = _count(r, data)
        if n < 3:
            raise ValueError("ring with fewer than 3 points")
        ring = []
        for _ in range(n):
            x += r.zigzag()
            y += r.zigzag()
            if abs(x) > 180 * k or abs(y) > 90 * k:
                raise ValueError("coordinate out of range")
            ring.append((x / k, y / k))
        ring_list.append(ring)
    geometries = []
    for _ in range(_count(r, data)):
        refs = [r.varint() for _ in range(_count(r, data))]
        if any(i >= len(ring_list) for i in refs):
            raise ValueError("bad ring index")
        geometries.append([ring_list[i] for i in refs])
    records = []
    for _ in range(_count(r, data)):
        name, start, end, geom = r.varint(), r.zigzag(), r.zigzag(), r.varint()
        llon, llat, tag = r.zigzag() / k, r.zigzag() / k, r.varint()
        if name >= len(names) or geom >= len(geometries) or start > end:
            raise ValueError(f"bad record {name, start, end, geom}")
        if abs(llon) > 180 or abs(llat) > 90:
            raise ValueError("label out of range")
        records.append((name, start, end, geom, (llon, llat), tag))
    if r.pos != len(data):
        raise ValueError("trailing bytes")
    return scale, digits, names, geometries, records


def _count(r, data):
    n = r.varint()
    # Every counted item takes at least one byte.
    if n > len(data) - r.pos:
        raise ValueError("count exceeds data size")
    return n
