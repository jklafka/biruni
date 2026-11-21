#!/usr/bin/env python3
"""Tests for fetch_space.py, without the network: canned Horizons and SBDB
answers. `--write-fixture` rewrites scripts/testdata/tiny_space.bin, which
src/space.rs also decodes."""
import datetime as dt
import math
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import fetch_space as fs  # noqa: E402

FIXTURE = Path(__file__).parent / "testdata" / "tiny_space.bin"
DATE = dt.datetime(2026, 11, 20, tzinfo=dt.timezone.utc)

VECTORS = """\
*******************************************************************************
 Revised: Jan 01, 2026            Voyager 1 (spacecraft)               -31
$$SOE
2461364.500000000, A.D. 2026-Nov-20 00:00:00.0000, -3.1E+01, -1.3E+02,  9.8E+01,
2461365.500000000, A.D. 2026-Nov-21 00:00:00.0000, -3.2E+01, -1.31E+02, 9.81E+01,
2461366.500000000, A.D. 2026-Nov-22 00:00:00.0000, -3.3E+01, -1.32E+02, 9.82E+01,
$$EOE
*******************************************************************************
"""

ELEMENTS = """\
            JDTDB,            Calendar Date (TDB),                     EC,                     QR,                     IN,                     OM,                      W,                     Tp,                      N,                     MA,                     TA,                      A,                     AD,                     PR,
**************************************************************************************************************************************************************
$$SOE
2461364.500000000, A.D. 2026-Nov-20 00:00:00.0000,  9.679427744190555E-01,  5.725079770538561E-01,  1.621577670748777E+02,  5.952134533622812E+01,  1.124196652497173E+02,  2.474052322638840E+06,  1.30E-02,  1.0E+02,  1.7E+02,  1.78E+01,  3.5E+01,  2.7E+04,
$$EOE
"""

AFTER = """\
No ephemeris for target "MPO-BepiColombo (Spacecraft)" after A.D. 2027-APR-11 00:01:09.1855 TDB
"""
PRIOR = """\
No ephemeris for target "Hera (spacecraft)" prior to A.D. 2026-DEC-01 12:30:00.0000 TDB
"""


def craft_samples(n, start=1.79e9, r=1.0):
    return (start, 86_400.0, [(r * math.cos(i / 10), r * math.sin(i / 10), 0.01 * i) for i in range(n)])


def fixture_objects():
    """Five spacecraft and five small bodies (the minimum `check` accepts),
    with simple, recognizable values for src/space.rs's tests."""
    objects = [
        # A circle of radius 1 au in the ecliptic, a day a sample, starting
        # 2026-01-01 00:00 UTC (1767225600).
        (0, "Circler", (1_767_225_600.0, 86_400.0,
                        [(math.cos(i * 0.1), math.sin(i * 0.1), 0.0) for i in range(5)])),
        (0, "Voyager 1", (1_767_225_600.0, 86_400.0, [(-31.0, -130.0, 98.0), (-31.0, -130.1, 98.1)])),
        (0, "Fast", (1_767_225_600.0, 21_600.0, [(0.1, 0.0, 0.0), (0.0, 0.1, 0.0), (-0.1, 0.0, 0.0)])),
        (0, "Sondé", (1_767_225_600.0, 86_400.0, [(1.5, 0.0, 0.0), (1.5, 0.01, 0.0)])),
        (0, "Late", (1_798_761_600.0, 86_400.0, [(2.0, 0.0, 0.0), (2.0, 0.01, 0.0)])),
        # Elements: a circular orbit at 1 au in the ecliptic with perihelion
        # (on the equinox direction) at J2000.0.
        (1, "Circle", (2_451_545.0, 0.0, 1.0, 0.0, 0.0, 0.0, 2_451_545.0)),
        (1, "Ceres", (2_461_364.5, 0.0796, 2.5455, 10.588, 80.25, 73.30, 2_461_599.95)),
        (2, "1P/Halley", (2_461_364.5, 0.96794, 0.57251, 162.158, 59.521, 112.420, 2_474_052.32)),
        # A hyperbolic comet (3I/ATLAS) and a parabola.
        (2, "3I/ATLAS", (2_461_000.5, 6.139, 1.3564, 175.113, 322.155, 128.006, 2_460_977.98)),
        (2, "Parabola", (2_451_545.0, 1.0, 1.0, 0.0, 0.0, 0.0, 2_451_545.0)),
    ]
    return 1_779_235_200.0, objects  # as of 2026-05-20


def fixture_bytes():
    as_of, objects = fixture_objects()
    return fs.encode(as_of, objects)


class Parsing(unittest.TestCase):
    def test_vectors(self):
        rows = fs.parse_vectors(VECTORS)
        self.assertEqual(len(rows), 3)
        self.assertAlmostEqual(rows[0][0], 1_795_132_800.0, places=3)  # 2026-11-20 UTC
        self.assertEqual(rows[1][1:], (-32.0, -131.0, 98.1))

    def test_elements(self):
        jd, e, q, inc, node, peri, tp = fs.parse_elements(ELEMENTS)
        self.assertEqual(jd, 2461364.5)
        self.assertAlmostEqual(e, 0.9679427744190555)
        self.assertAlmostEqual(inc, 162.1577670748777)
        self.assertAlmostEqual(tp, 2474052.32263884)
        with self.assertRaises(fs.FetchError):
            fs.parse_elements("no table here")

    def test_coverage_limits(self):
        which, when = fs.coverage_limit(AFTER)
        self.assertEqual((which, when), ("after", dt.datetime(2027, 4, 11, 0, 1, tzinfo=dt.timezone.utc)))
        which, when = fs.coverage_limit(PRIOR)
        self.assertEqual((which, when), ("prior", dt.datetime(2026, 12, 1, 12, 30, tzinfo=dt.timezone.utc)))
        self.assertIsNone(fs.coverage_limit("Unknown target"))


def fake_horizons(first_ok, last_ok, step_hours=24):
    """A Horizons stand-in with an ephemeris from `first_ok` to `last_ok`."""
    calls = []

    def query(params):
        calls.append(params)
        start = dt.datetime.strptime(params["START_TIME"].strip("'"), "%Y-%m-%d %H:%M").replace(tzinfo=dt.timezone.utc)
        stop = dt.datetime.strptime(params["STOP_TIME"].strip("'"), "%Y-%m-%d %H:%M").replace(tzinfo=dt.timezone.utc)
        # Horizons names the month in capitals, as in AFTER.
        def when(t):
            return t.strftime("%Y-%b-%d %H:%M").upper()
        if stop > last_ok:
            return f'No ephemeris for target "X" after A.D. {when(last_ok)}:00.0000 TDB'
        if start < first_ok:
            return f'No ephemeris for target "X" prior to A.D. {when(first_ok)}:00.0000 TDB'
        lines = []
        t = start
        while t <= stop:
            jd = t.timestamp() / 86_400 + 2440587.5
            lines.append(f"{jd:.9f}, A.D. x, 1.0E+00, 2.0E+00, 3.0E+00,")
            t += dt.timedelta(hours=step_hours)
        return "$$SOE\n" + "\n".join(lines) + "\n$$EOE\n"

    return query, calls


class Spacecraft(unittest.TestCase):
    def test_full_window(self):
        query, calls = fake_horizons(DATE - dt.timedelta(days=999), DATE + dt.timedelta(days=999))
        start, step, pts = fs.fetch_spacecraft("-31", 24, DATE, query=query)
        self.assertEqual(len(calls), 1)
        self.assertEqual(step, 86_400.0)
        self.assertEqual(len(pts), fs.BEFORE_DAYS + fs.AFTER_DAYS + 1)
        self.assertAlmostEqual(start, (DATE - dt.timedelta(days=fs.BEFORE_DAYS)).timestamp(), places=2)

    def test_clipped_to_the_ephemeris(self):
        last = DATE + dt.timedelta(days=100, hours=3)
        first = DATE - dt.timedelta(days=10, hours=5)
        query, calls = fake_horizons(first, last)
        start, step, pts = fs.fetch_spacecraft("-121", 24, DATE, query=query)
        self.assertEqual(len(calls), 3)
        self.assertGreaterEqual(start, first.timestamp())
        self.assertLessEqual(start + step * (len(pts) - 1), last.timestamp())
        # Whole steps from the original start: the grid is kept.
        self.assertEqual((start - (DATE - dt.timedelta(days=30)).timestamp()) % step, 0)

    def test_no_ephemeris_near_the_date(self):
        query, _ = fake_horizons(DATE + dt.timedelta(days=500), DATE + dt.timedelta(days=900))
        with self.assertRaises(fs.FetchError):
            fs.fetch_spacecraft("-1", 24, DATE, query=query)


class Comets(unittest.TestCase):
    def test_brightest_near_perihelion(self):
        seen = {}

        def get_json(params):
            seen.update(params)
            return {"data": [
                ["     C/2025 R3 (PANSTARRS)", "2025 R3", "C", "11.7"],
                ["  210P/Christensen", "210P", None, "14.2"],
                ["     C/2024 E1 (Wierzchos)", "2024 E1", "C", "11.6"],
                ["     P/2026 R2 (Leonard)", "2026 R2", "P", "18.9"],
                ["     C/2099 X1 (Nobody)", "2099 X1", "C", None],
            ]}

        got = fs.bright_comets(DATE, n=3, get_json=get_json)
        self.assertEqual(got, [
            ("DES=C/2024 E1;", "C/2024 E1 (Wierzchos)"),
            ("DES=C/2025 R3;", "C/2025 R3 (PANSTARRS)"),
            ("DES=210P;CAP", "210P/Christensen"),
        ])
        self.assertIn("q|LT|1.5", seen["sb-cdata"])
        self.assertEqual(seen["sb-kind"], "c")


class FromLive(unittest.TestCase):
    """--from-live keeps the newer of the committed and the published file."""

    def run_with(self, published, current):
        import io
        import json
        import tempfile
        from unittest import mock

        def urlopen(req, timeout=None):
            url = req.full_url
            if url.endswith("manifest.json"):
                if published is None:
                    raise OSError("404")
                return io.BytesIO(json.dumps({"files": {"space.bin": "https://r2/space.x.bin"}}).encode())
            return io.BytesIO(published)

        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / "space.bin"
            out.write_bytes(current)
            with mock.patch.object(fs.urllib.request, "urlopen", urlopen):
                copied = fs.from_live("https://site/data/manifest.json", str(out))
            return copied, out.read_bytes()

    def test_newer_live_file_wins(self):
        as_of, objects = fixture_objects()
        old, new = fs.encode(as_of, objects), fs.encode(as_of + 86_400, objects)
        self.assertEqual(self.run_with(new, old), (True, new))
        self.assertEqual(self.run_with(old, new), (False, new))

    def test_failures_keep_the_committed_file(self):
        as_of, objects = fixture_objects()
        current = fs.encode(as_of, objects)
        self.assertEqual(self.run_with(None, current), (False, current))
        self.assertEqual(self.run_with(b"SPCE junk", current), (False, current))


class Format(unittest.TestCase):
    def test_round_trip(self):
        as_of, objects = fixture_objects()
        back_as_of, back = fs.decode(fs.encode(as_of, objects))
        self.assertEqual(back_as_of, as_of)
        self.assertEqual([(k, n) for k, n, _ in back], [(k, n) for k, n, _ in objects])
        for (k, _, a), (_, _, b) in zip(objects, back):
            if k == 0:
                self.assertEqual(a[:2], b[:2])
                for p, q in zip(a[2], b[2]):
                    for x, y in zip(p, q):
                        # f32: about 7 significant digits.
                        self.assertLess(abs(x - y), 1e-6 * max(1.0, abs(x)))
            else:
                self.assertEqual(a, b)

    def test_bad_input(self):
        data = fixture_bytes()
        for bad in (b"", data[:-1], data + b"\0", b"XXXX" + data[4:], data[:4] + b"\x02" + data[5:]):
            with self.assertRaises(ValueError):
                fs.decode(bad)
        # An unknown kind.
        k = fs.HEADER.size
        with self.assertRaises(ValueError):
            fs.decode(data[:k] + b"\x07" + data[k + 1:])

    def test_check(self):
        self.assertEqual(fs.check(fixture_bytes()), [])
        as_of, objects = fixture_objects()
        self.assertIn("only", fs.check(fs.encode(as_of, objects[:3]))[0])
        far = list(objects)
        far[0] = (0, "Far", (1.7e9, 86_400.0, [(500.0, 0.0, 0.0), (500.0, 0.0, 0.0)]))
        self.assertTrue(any("au" in p for p in fs.check(fs.encode(as_of, far))))
        dup = list(objects)
        dup[1] = (0, "Circler", dup[1][2])
        self.assertIn("duplicate names", fs.check(fs.encode(as_of, dup)))
        stale = fs.check(fixture_bytes(), now=as_of + 40 * 86_400, max_age_days=30)
        self.assertEqual(stale, ["older than 30 days"])
        self.assertEqual(fs.check(b"junk"), ["too short"])

    def test_committed_files_are_valid(self):
        data = (Path(__file__).parent.parent / "assets" / "data" / "space.bin").read_bytes()
        self.assertEqual(fs.check(data), [])
        self.assertEqual(FIXTURE.read_bytes(), fixture_bytes(),
                         "run: python3 scripts/test_fetch_space.py --write-fixture")


if __name__ == "__main__":
    if sys.argv[1:] == ["--write-fixture"]:
        FIXTURE.write_bytes(fixture_bytes())
        print(f"wrote {FIXTURE}")
    else:
        unittest.main()
