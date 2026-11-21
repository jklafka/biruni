#!/usr/bin/env python3
"""Build assets/data/space.bin: spacecraft and small bodies for the solar
system view, from NASA JPL's Horizons system and Small-Body Database.

The deploy workflow runs this on a schedule (and on request) to refresh the
published file; pushes to main reuse the live file; the committed copy is a
fixed snapshot for development, CI and the visual tests. The browser never
asks JPL itself. Requests go one at a time with a pause between them, as
the APIs ask of scripts. Stdlib only.

    python3 scripts/fetch_space.py OUT.bin [--date YYYY-MM-DD]
    python3 scripts/fetch_space.py OUT.bin --from-live MANIFEST_URL
    python3 scripts/fetch_space.py --check FILE.bin

What it holds:

- Spacecraft (SPACECRAFT): heliocentric positions sampled from 30 days
  before the date to 365 days after (a step per craft; shorter where the
  spacecraft moves fast near the Sun), clipped to the span Horizons has.
- Small bodies (SMALL_BODIES, plus the few brightest comets with a
  perihelion near the date and q < 1.5 au, found with the SBDB query API):
  osculating orbital elements at the date, which the app propagates as a
  two-body orbit (good for years near the date, not for close planetary
  encounters).

Format "SPCE" version 1, little-endian (read by src/space.rs, checked by
scripts/validate_data.py). Positions and elements are heliocentric, in the
J2000 ecliptic frame (x towards the equinox, z towards the ecliptic pole).

    0   4   magic b"SPCE"
    4   1   version (1)
    5   3   reserved (0)
    8   8   f64 as-of time (UTC Unix seconds): when the data was fetched
    16  2   u16 object count
    18  ... objects:
            u8  kind: 0 spacecraft, 1 asteroid or dwarf planet, 2 comet
            u8  name length, UTF-8 name
            kind 0:
              f64 time of the first sample (UTC Unix seconds)
              f32 seconds between samples
              u16 sample count N
              N x 3 f32: x, y, z in au
            kinds 1 and 2:
              f64 epoch (Julian date, TDB) of the elements
              6 f64: eccentricity, perihelion distance (au), inclination,
                     longitude of the ascending node, argument of
                     perihelion (degrees), time of perihelion (JD TDB)
"""
import argparse
import datetime as dt
import json
import math
import struct
import sys
import time
import urllib.parse
import urllib.request

HORIZONS = "https://ssd.jpl.nasa.gov/api/horizons.api"
SBDB_QUERY = "https://ssd-api.jpl.nasa.gov/sbdb_query.api"
USER_AGENT = "data-refresh"
TIMEOUT = 90
PAUSE = 1.0  # seconds between requests

MAGIC = b"SPCE"
VERSION = 1
HEADER = struct.Struct("<4sB3xdH")
SAMPLES = struct.Struct("<dfH")
ELEMENTS = struct.Struct("<7d")
KINDS = {0: "spacecraft", 1: "asteroid", 2: "comet"}

BEFORE_DAYS = 30
AFTER_DAYS = 365
DAY = 86_400.0

# Horizons id, name, sample step in hours.
SPACECRAFT = [
    ("-31", "Voyager 1", 24),
    ("-32", "Voyager 2", 24),
    ("-98", "New Horizons", 24),
    ("-96", "Parker Solar Probe", 6),
    ("-144", "Solar Orbiter", 12),
    ("-121", "BepiColombo", 12),
    ("-61", "Juno", 24),
    ("-170", "JWST", 24),
    ("-159", "Europa Clipper", 24),
    ("-28", "JUICE", 24),
    ("-255", "Psyche", 24),
    ("-49", "Lucy", 24),
    ("-91", "Hera", 24),
    ("-64", "OSIRIS-APEX", 24),
]

# Horizons command, name, kind.
SMALL_BODIES = [
    ("1;", "Ceres", 1),
    ("2;", "Pallas", 1),
    ("4;", "Vesta", 1),
    ("134340;", "Pluto", 1),
    ("136199;", "Eris", 1),
    ("136108;", "Haumea", 1),
    ("136472;", "Makemake", 1),
    ("101955;", "Bennu", 1),
    ("99942;", "Apophis", 1),
    ("DES=1P;CAP", "1P/Halley", 2),
    ("DES=2P;CAP", "2P/Encke", 2),
    ("DES=67P;CAP", "67P/Churyumov-Gerasimenko", 2),
    ("DES=C/2025 N1;", "3I/ATLAS", 2),
]
# Comets near perihelion added by the SBDB query, at most this many.
BRIGHT_COMETS = 5

# At most this many objects may fail before the whole refresh fails (a
# spacecraft whose ephemeris ended, a renamed comet).
MAX_FAILURES = 3


class FetchError(Exception):
    pass


def get(url, params):
    full = url + "?" + urllib.parse.urlencode(params)
    req = urllib.request.Request(full, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
        body = resp.read().decode("utf-8")
    time.sleep(PAUSE)
    return body


def horizons(params):
    """The text result of a Horizons query (its JSON can hold raw newlines)."""
    doc = json.loads(get(HORIZONS, dict(format="json", **params)), strict=False)
    if "error" in doc and "result" not in doc:
        raise FetchError(doc["error"])
    return doc.get("result", "")


def table(result):
    """Rows (lists of stripped strings) between $$SOE and $$EOE."""
    if "$$SOE" not in result:
        raise FetchError(last_lines(result))
    body = result.split("$$SOE", 1)[1].split("$$EOE", 1)[0]
    return [[c.strip() for c in line.split(",")] for line in body.strip().splitlines() if line.strip()]


def last_lines(text, n=3):
    lines = [line.strip() for line in text.strip().splitlines() if line.strip()]
    return " / ".join(lines[-n:])[:400]


MONTHS = {m: i for i, m in enumerate(
    ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"], 1)}


def coverage_limit(result):
    """("after" | "prior", datetime) from Horizons' message for a span it
    has no ephemeris for, e.g. 'No ephemeris for target "X" after A.D.
    2027-APR-11 00:01:09.1855 TDB', or None."""
    import re

    m = re.search(r"No ephemeris for target .*? (after|prior to) A\.D\. (\d{4})-([A-Z]{3})-(\d{2})"
                  r"(?: (\d{2}):(\d{2}))?", result)
    if not m:
        return None
    which = "after" if m.group(1) == "after" else "prior"
    hh, mm = int(m.group(5) or 0), int(m.group(6) or 0)
    when = dt.datetime(int(m.group(2)), MONTHS[m.group(3)], int(m.group(4)), hh, mm,
                       tzinfo=dt.timezone.utc)
    return which, when


def fmt_time(t):
    return t.strftime("%Y-%m-%d %H:%M")


def vectors_query(cmd, start, stop, step_hours):
    return {
        "COMMAND": f"'{cmd}'",
        "EPHEM_TYPE": "VECTORS",
        "CENTER": "'500@10'",
        "REF_PLANE": "ECLIPTIC",
        "REF_SYSTEM": "ICRF",
        "VEC_TABLE": "1",
        "CSV_FORMAT": "YES",
        "OUT_UNITS": "AU-D",
        "TIME_TYPE": "UT",
        "OBJ_DATA": "NO",
        "START_TIME": f"'{fmt_time(start)}'",
        "STOP_TIME": f"'{fmt_time(stop)}'",
        "STEP_SIZE": f"'{step_hours} h'",
    }


def parse_vectors(result):
    """[(unix seconds, x, y, z)] from a VECTORS table with TIME_TYPE=UT."""
    out = []
    for row in table(result):
        jd = float(row[0])
        out.append(((jd - 2440587.5) * DAY, float(row[2]), float(row[3]), float(row[4])))
    return out


def fetch_spacecraft(cmd, step_hours, date, query=horizons):
    """Samples of a spacecraft around `date`, clipped to its ephemeris span.
    Returns (start unix, step seconds, [(x, y, z)])."""
    start = date - dt.timedelta(days=BEFORE_DAYS)
    stop = date + dt.timedelta(days=AFTER_DAYS)
    for _ in range(3):
        result = query(vectors_query(cmd, start, stop, step_hours))
        if "$$SOE" in result:
            rows = parse_vectors(result)
            break
        limit = coverage_limit(result)
        if limit is None:
            raise FetchError(last_lines(result))
        which, when = limit
        # Whole steps inside the span, a step clear of its edge.
        step = dt.timedelta(hours=step_hours)
        if which == "after":
            stop = when - step
        else:
            start = start + step * math.ceil((when - start) / step + 1)
        if stop <= start:
            raise FetchError(f"no ephemeris near {date:%Y-%m-%d}")
    else:
        raise FetchError("ephemeris span not found")
    if len(rows) < 2:
        raise FetchError("fewer than two samples")
    step = step_hours * 3600.0
    for (t0, *_), (t1, *_) in zip(rows, rows[1:]):
        if abs(t1 - t0 - step) > 1.0:
            raise FetchError(f"uneven samples at {t0}")
    return rows[0][0], step, [r[1:] for r in rows]


def elements_query(cmd, date):
    jd = date.timestamp() / DAY + 2440587.5
    return {
        "COMMAND": f"'{cmd}'",
        "EPHEM_TYPE": "ELEMENTS",
        "CENTER": "'500@10'",
        "REF_PLANE": "ECLIPTIC",
        "REF_SYSTEM": "ICRF",
        "CSV_FORMAT": "YES",
        "OUT_UNITS": "AU-D",
        "OBJ_DATA": "NO",
        "TLIST": f"'{jd:.1f}'",
    }


def parse_elements(result):
    """(epoch JD, e, q, i, node, peri, tp) from an ELEMENTS table: columns
    JDTDB, date, EC, QR, IN, OM, W, Tp, ..."""
    row = table(result)[0]
    jd, e, q, inc, node, peri, tp = (float(row[i]) for i in (0, 2, 3, 4, 5, 6, 7))
    if not (e >= 0 and q > 0 and all(math.isfinite(v) for v in (jd, inc, node, peri, tp))):
        raise FetchError(f"bad elements {row[:8]}")
    return jd, e, q, inc, node, peri, tp


def bright_comets(date, n=BRIGHT_COMETS, get_json=None):
    """(Horizons command, display name) of the n brightest comets (by the
    SBDB total magnitude parameter M1) with q < 1.5 au and a perihelion from
    120 days before `date` to 240 days after."""
    jd = date.timestamp() / DAY + 2440587.5
    params = {
        "fields": "full_name,pdes,prefix,M1",
        "sb-kind": "c",
        "sb-cdata": json.dumps({"AND": ["q|LT|1.5", f"tp|RG|{jd - 120:.1f}|{jd + 240:.1f}"]}),
    }
    doc = (get_json or (lambda p: json.loads(get(SBDB_QUERY, p))))(params)
    rows = []
    for full, pdes, prefix, m1 in doc.get("data", []):
        try:
            mag = float(m1)
        except (TypeError, ValueError):
            continue
        rows.append((mag, full.strip(), pdes, prefix or ""))
    rows.sort()
    out = []
    for _, full, pdes, prefix in rows[:n]:
        # Numbered periodic comets are "210P", others "C/2025 R3".
        des = pdes if pdes.endswith("P") and pdes[:-1].isdigit() else f"{prefix}/{pdes}"
        cmd = f"DES={des};CAP" if des.endswith("P") else f"DES={des};"
        out.append((cmd, full))
    return out


def encode(as_of, objects):
    """objects: (kind, name, payload) with payload (start, step, [(x, y, z)])
    for spacecraft and the seven element values for the others."""
    out = bytearray(HEADER.pack(MAGIC, VERSION, as_of, len(objects)))
    for kind, name, payload in objects:
        raw = name.encode("utf-8")[:255]
        out += struct.pack("<BB", kind, len(raw)) + raw
        if kind == 0:
            start, step, pts = payload
            out += SAMPLES.pack(start, step, len(pts))
            for p in pts:
                out += struct.pack("<3f", *p)
        else:
            out += ELEMENTS.pack(*payload)
    return bytes(out)


def decode(data):
    """(as_of, [(kind, name, payload)]); raises ValueError on bad input."""
    if len(data) < HEADER.size:
        raise ValueError("too short")
    magic, version, as_of, count = HEADER.unpack_from(data, 0)
    if magic != MAGIC or version != VERSION:
        raise ValueError(f"not SPCE v1: {magic!r} {version}")
    pos = HEADER.size
    objects = []

    def take(n):
        nonlocal pos
        if pos + n > len(data):
            raise ValueError("truncated")
        chunk = data[pos:pos + n]
        pos += n
        return chunk

    for _ in range(count):
        kind, length = struct.unpack("<BB", take(2))
        if kind not in KINDS:
            raise ValueError(f"unknown kind {kind}")
        name = take(length).decode("utf-8")
        if kind == 0:
            start, step, n = SAMPLES.unpack(take(SAMPLES.size))
            pts = [struct.unpack("<3f", take(12)) for _ in range(n)]
            objects.append((kind, name, (start, step, pts)))
        else:
            objects.append((kind, name, ELEMENTS.unpack(take(ELEMENTS.size))))
    if pos != len(data):
        raise ValueError(f"{len(data) - pos} trailing bytes")
    return as_of, objects


def check(data, now=None, max_age_days=None):
    """Problems with a space.bin (empty when fine)."""
    try:
        as_of, objects = decode(data)
    except (ValueError, UnicodeDecodeError, struct.error) as e:
        return [str(e)]
    problems = []
    if not math.isfinite(as_of) or as_of < 1.7e9:
        problems.append(f"as-of time {as_of}")
    if max_age_days is not None and now is not None and now - as_of > max_age_days * DAY:
        problems.append(f"older than {max_age_days} days")
    craft = sum(1 for k, _, _ in objects if k == 0)
    if craft < 5 or len(objects) - craft < 5:
        problems.append(f"only {craft} spacecraft and {len(objects) - craft} small bodies")
    names = [n for _, n, _ in objects]
    if len(set(names)) != len(names):
        problems.append("duplicate names")
    for kind, name, payload in objects:
        if not name:
            problems.append("empty name")
        if kind == 0:
            start, step, pts = payload
            if not (len(pts) >= 2 and 600 <= step <= 30 * DAY and math.isfinite(start)):
                problems.append(f"{name}: {len(pts)} samples every {step} s")
            for p in pts:
                r = math.sqrt(sum(c * c for c in p))
                if not (math.isfinite(r) and 0.001 < r < 300):
                    problems.append(f"{name}: sample at {r} au")
                    break
        else:
            jd, e, q, inc, node, peri, tp = payload
            if not (0 <= e < 20 and 0 < q < 200 and 0 <= inc <= 180 and math.isfinite(tp + jd + node + peri)):
                problems.append(f"{name}: elements {payload}")
    return problems


def fetch_all(date):
    objects, failures = [], []
    for cmd, name, step in SPACECRAFT:
        try:
            objects.append((0, name, fetch_spacecraft(cmd, step, date)))
            print(f"{name}: {len(objects[-1][2][2])} samples", file=sys.stderr)
        except (FetchError, OSError, ValueError) as e:
            failures.append(f"{name}: {e}")
    try:
        comets = bright_comets(date)
    except (OSError, ValueError) as e:
        failures.append(f"comet query: {e}")
        comets = []
    known = {cmd for cmd, _, _ in SMALL_BODIES}
    bodies = SMALL_BODIES + [(cmd, name, 2) for cmd, name in comets if cmd not in known]
    for cmd, name, kind in bodies:
        try:
            objects.append((kind, name, parse_elements(horizons(elements_query(cmd, date)))))
            print(f"{name}: elements", file=sys.stderr)
        except (FetchError, OSError, ValueError, IndexError) as e:
            failures.append(f"{name}: {e}")
    for f in failures:
        print(f"warning: {f}", file=sys.stderr)
    if len(failures) > MAX_FAILURES:
        raise SystemExit(f"{len(failures)} objects failed; not writing a thin file")
    return objects


def from_live(manifest_url, out_path):
    """Copy the published space.bin over `out_path` if it is newer."""
    from urllib.parse import urljoin

    req = urllib.request.Request(manifest_url, headers={"User-Agent": USER_AGENT,
                                                        "Origin": "https://biruni.jklafka.cloud"})
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
            manifest = json.loads(r.read())
        path = manifest["files"]["space.bin"]
    except (OSError, ValueError, KeyError) as e:
        print(f"no live space.bin ({e}); keeping {out_path}", file=sys.stderr)
        return False
    req = urllib.request.Request(urljoin(manifest_url, path), headers=req.headers)
    with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
        live = r.read()
    if check(live):
        raise SystemExit(f"the live space.bin is broken: {check(live)}")
    with open(out_path, "rb") as f:
        current = f.read()
    if decode(live)[0] <= decode(current)[0]:
        print(f"the live space.bin is not newer; keeping {out_path}", file=sys.stderr)
        return False
    with open(out_path, "wb") as f:
        f.write(live)
    print(f"copied the live space.bin (as of {decode(live)[0]:.0f})", file=sys.stderr)
    return True


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("out", nargs="?", help="output file")
    ap.add_argument("--date", help="center date YYYY-MM-DD (default: today, UTC)")
    ap.add_argument("--from-live", metavar="MANIFEST_URL",
                    help="copy the published file instead, if it is newer than OUT")
    ap.add_argument("--check", metavar="FILE", help="validate a file and exit")
    args = ap.parse_args(argv)
    if args.check:
        with open(args.check, "rb") as f:
            problems = check(f.read())
        for p in problems:
            print(f"{args.check}: {p}", file=sys.stderr)
        return 1 if problems else 0
    if not args.out:
        ap.error("OUT is required")
    if args.from_live:
        from_live(args.from_live, args.out)
        return 0
    now = dt.datetime.now(dt.timezone.utc)
    if args.date:
        date = dt.datetime.strptime(args.date, "%Y-%m-%d").replace(tzinfo=dt.timezone.utc)
        as_of = date.timestamp()
    else:
        date = now.replace(hour=0, minute=0, second=0, microsecond=0)
        as_of = float(int(now.timestamp()))
    data = encode(as_of, fetch_all(date))
    problems = check(data)
    if problems:
        raise SystemExit("; ".join(problems))
    with open(args.out, "wb") as f:
        f.write(data)
    print(f"wrote {args.out}: {len(data)} bytes", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
