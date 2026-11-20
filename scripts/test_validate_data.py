#!/usr/bin/env python3
"""Unit tests for scripts/validate_data.py (stdlib only).

Run: python3 scripts/test_validate_data.py (scripts/check-data.sh runs it too).
"""
import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))

import convert_geojson as conv  # noqa: E402
import validate_data as vd  # noqa: E402


def mapd(shapes, version=1):
    """Encode shapes ([(points, closed)] in degrees) like convert_geojson."""
    return conv.MAGIC + bytes([version]) + _body(shapes)


def _body(shapes):
    out = bytearray(conv.varint(len(shapes)))
    px = py = 0
    for pts, closed in shapes:
        out += conv.varint(len(pts) << 1 | int(closed))
        for lon, lat in pts:
            x, y = round(lon * conv.SCALE), round(lat * conv.SCALE)
            out += conv.varint(conv.zigzag(x - px)) + conv.varint(conv.zigzag(y - py))
            px, py = x, y
    return bytes(out)


LINE = [([(0.0, 0.0), (10.0, 5.0), (20.0, 10.0)], False)] * 10
TSV = "MAPSLABELS1\nC\tA\t10.00\t20.00\t0\t5\t1.0\nP\tB\t-5.00\t-170.00\t1\t9\t0.0\n"


def make_dir(root, files):
    """A data dir like hash_data.py writes. files: name -> bytes."""
    root = Path(root)
    (root / "files").mkdir(parents=True)
    entries = {}
    for name, body in files.items():
        rel = vd.hashed_path(name, body)
        (root / rel).write_bytes(body)
        entries[name] = rel
    (root / "manifest.json").write_text(json.dumps({"version": 1, "files": entries}))
    return root


class CommittedData(unittest.TestCase):
    def test_bins_decode_with_the_converters_feature_counts(self):
        for src in sorted((ROOT / "data-src").glob("*.geojson")):
            out = ROOT / "assets" / "data" / (src.stem + ".bin")
            if not out.exists():
                continue
            gj = json.loads(src.read_text(encoding="utf-8"))
            if "admin_0_countries" in src.stem:
                want = sum(
                    1
                    for f in gj["features"]
                    if f.get("geometry") and f["geometry"]["type"] in ("Polygon", "MultiPolygon")
                )
                got = vd.decode_countries(out.read_bytes())
            else:
                want = len(list(conv.shapes_of(gj)))
                got = vd.decode_shapes(out.read_bytes())
            self.assertEqual(got, want, src.name)
            self.assertGreater(got, 0)

    def test_every_committed_file_passes(self):
        data = ROOT / "assets" / "data"
        d = make_dir(tempfile.mkdtemp(), {p.name: p.read_bytes() for p in data.iterdir()})
        self.assertEqual(vd.validate(d), [])


class Validate(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)

    def dir(self, files, name=None):
        self.count = getattr(self, "count", 0) + 1
        return make_dir(self.tmp / (name or f"d{self.count}"), files)

    def good(self):
        return {"land.bin": mapd(LINE), "labels.tsv": TSV.encode()}

    def problems(self, files, **kw):
        return vd.validate(self.dir(files), **kw)

    def assertProblem(self, problems, text):
        self.assertTrue(any(text in p for p in problems), f"{text!r} not in {problems}")

    def test_good_dir_passes(self):
        self.assertEqual(self.problems(self.good()), [])

    def test_bad_hash(self):
        d = self.dir(self.good())
        m = json.loads((d / "manifest.json").read_text())
        path = d / m["files"]["land.bin"]
        path.write_bytes(path.read_bytes() + b"\0")
        self.assertProblem(vd.validate(d), "content hash")

    def test_stray_file(self):
        d = self.dir(self.good())
        (d / "files" / "extra.bin").write_bytes(b"x")
        self.assertProblem(vd.validate(d), "files/extra.bin: not listed")

    def test_missing_listed_file(self):
        d = self.dir(self.good())
        m = json.loads((d / "manifest.json").read_text())
        (d / m["files"]["labels.tsv"]).unlink()
        self.assertProblem(vd.validate(d), "is missing")

    def test_bad_manifest(self):
        d = self.dir(self.good())
        (d / "manifest.json").write_text("{")
        self.assertProblem(vd.validate(d), "cannot read")
        (d / "manifest.json").write_text('{"version": 2, "files": {}}')
        self.assertProblem(vd.validate(d), "unsupported version")

    def test_unsafe_paths(self):
        d = self.dir(self.good())
        for rel in ("/etc/passwd", "../x.bin", "files/../../x.bin"):
            (d / "manifest.json").write_text(json.dumps({"version": 1, "files": {"land.bin": rel}}))
            self.assertProblem(vd.validate(d), "unsafe path")

    def test_oversize(self):
        old = vd.MAX_FILE_BYTES
        vd.MAX_FILE_BYTES = 10
        self.addCleanup(setattr, vd, "MAX_FILE_BYTES", old)
        self.assertProblem(self.problems(self.good()), "file limit")

    def test_total_over_limit(self):
        old = vd.MAX_TOTAL_BYTES
        vd.MAX_TOTAL_BYTES = 10
        self.addCleanup(setattr, vd, "MAX_TOTAL_BYTES", old)
        self.assertProblem(self.problems(self.good()), "total")

    def test_truncated_bin(self):
        files = self.good()
        files["land.bin"] = files["land.bin"][:-3]
        self.assertProblem(self.problems(files), "land.bin")

    def test_star_catalog(self):
        stars = (ROOT / "scripts" / "testdata" / "tiny_stars.bin").read_bytes()
        files = self.good()
        files["stars.bin"] = stars
        self.assertEqual(self.problems(files), [])
        for bad, text in ((stars[:-1], "star catalog"), (stars[:12] + b"\0\0\0\0" + stars[16:], "star catalog")):
            files["stars.bin"] = bad
            self.assertProblem(self.problems(files), text)
        # Out of order: swap the brightest and the faintest of the 4 records.
        rec = [stars[16 + 10 * i:26 + 10 * i] for i in range(4)]
        swapped = stars[:16] + rec[3] + rec[1] + rec[2] + rec[0] + stars[56:]
        files["stars.bin"] = swapped
        self.assertProblem(self.problems(files), "sorted")

    def test_constellations(self):
        figs = (ROOT / "scripts" / "testdata" / "tiny_constellations.bin").read_bytes()
        files = self.good()
        files["constellations.bin"] = figs
        self.assertEqual(self.problems(files), [])
        files["constellations.bin"] = figs[:-2]
        self.assertProblem(self.problems(files), "constellations")

    def test_trailing_bytes(self):
        files = self.good()
        files["land.bin"] += b"\0"
        self.assertProblem(self.problems(files), "trailing bytes")

    def test_out_of_range_coordinate(self):
        files = self.good()
        files["land.bin"] = mapd([([(0.0, 0.0), (181.0, 0.0)], False)] * 10)
        self.assertProblem(self.problems(files), "out of lon/lat range")
        files["land.bin"] = mapd([([(0.0, 0.0), (0.0, 91.0)], False)] * 10)
        self.assertProblem(self.problems(files), "out of lon/lat range")

    def test_bad_version_byte(self):
        files = self.good()
        files["land.bin"] = mapd(LINE, version=2)
        self.assertProblem(self.problems(files), "unsupported map data version 2")

    def test_bad_magic_and_empty(self):
        files = self.good()
        files["land.bin"] = b"NOPE\x01\x01"
        self.assertProblem(self.problems(files), "not a map data file")
        files["land.bin"] = mapd([])
        self.assertProblem(self.problems(files), "no features")

    def test_unknown_extension(self):
        files = self.good()
        files["mystery.dat"] = b"x"
        self.assertProblem(self.problems(files), "unknown file type")

    def test_malformed_tsv(self):
        for bad, text in (
            (TSV.replace("\t5\t", "\t"), "columns"),
            (TSV.replace("10.00", "91.00"), "out of range"),
            (TSV.replace("20.00", "nan"), "out of range"),
            (TSV.replace("20.00", "x"), "not a number"),
            (TSV.replace("MAPSLABELS1", "OTHER"), "header"),
            (TSV.rstrip("\n"), "final newline"),
            ("MAPSLABELS1\n", "no features"),
        ):
            files = self.good()
            files["labels.tsv"] = bad.encode()
            self.assertProblem(self.problems(files), text)
        files = self.good()
        files["labels.tsv"] = b"MAPSLABELS1\n\xff\n"
        self.assertProblem(self.problems(files), "UTF-8")

    def test_webp(self):
        def webp(w, h):
            body = b"VP8 " + (14).to_bytes(4, "little") + b"\0" * 3 + b"\x9d\x01\x2a"
            body += w.to_bytes(2, "little") + h.to_bytes(2, "little") + b"\0" * 4
            return b"RIFF" + (len(body) + 4).to_bytes(4, "little") + b"WEBP" + body

        files = self.good()
        files["blue_marble.webp"] = webp(4096, 2048)
        self.assertEqual(self.problems(files), [])
        files["blue_marble.webp"] = webp(100, 100)
        self.assertProblem(self.problems(files), "expected 4096x2048")
        files["blue_marble.webp"] = b"RIFF" + b"\0" * 40
        self.assertProblem(self.problems(files), "not a WebP")
        files = self.good()
        files["terrain.webp"] = webp(4096, 2048)
        self.assertEqual(self.problems(files), [])
        files["terrain.webp"] = webp(2048, 1024)
        self.assertProblem(self.problems(files), "expected 4096x2048")
        files = self.good()
        files["night_lights.webp"] = webp(4096, 2048)
        self.assertEqual(self.problems(files), [])
        files["night_lights.webp"] = webp(4096, 4096)
        self.assertProblem(self.problems(files), "expected 4096x2048")

    def test_tilesets(self):
        sha = "ab" * 32
        good = {"layer": "imagery", "url": f"https://r2.example/files/imagery.{sha[:16]}.tiles",
                "sha256": sha, "size": 9_000_000, "min_level": 3, "max_level": 5,
                "tile_size": 256, "format": "webp"}

        def doc(*tilesets, **top):
            return json.dumps(dict({"version": 1, "tilesets": list(tilesets)}, **top)).encode()

        files = self.good()
        files["tilesets.json"] = doc()
        self.assertEqual(self.problems(files), [], "an empty list is valid")
        files["tilesets.json"] = doc(good, dict(good, layer="terrain", max_level=4))
        self.assertEqual(self.problems(files), [])
        for bad, text in (
            (b"{", "not valid JSON"),
            (doc(good, extra=1), "exactly 'version'"),
            (doc(good, version=2), "unsupported version"),
            (doc(dict(good, layer="pop")), "unknown layer"),
            (doc(good, good), "second tileset"),
            (doc(dict(good, sha256="AB" * 32)), "sha256"),
            (doc(dict(good, url="http://r2.example/x.tiles")), "https"),
            (doc(dict(good, url="https://r2.example/files/imagery.0000000000000000.tiles")), "sha256 prefix"),
            (doc(dict(good, min_level=6)), "bad levels"),
            (doc(dict(good, max_level=True)), "integers"),
            (doc(dict(good, tile_size=512)), "256px WebP"),
            (doc(dict(good, size=100)), "directory"),
            (doc({k: v for k, v in good.items() if k != "format"}), "want exactly the keys"),
        ):
            with self.subTest(text):
                files["tilesets.json"] = bad
                self.assertProblem(self.problems(files), text)

    def test_shrink_beyond_half(self):
        base = self.dir(self.good(), "base")
        small = self.good()
        small["land.bin"] = mapd(LINE[:4])  # 4 of 10 shapes
        d = self.dir(small, "new")
        self.assertProblem(vd.validate(d, baseline=base), "features, less than half")
        self.assertProblem(vd.validate(d, baseline=base), "bytes, less than half")
        self.assertEqual(vd.validate(d, baseline=base, allow_shrink=["land.bin"]), [])

    def test_shrink_within_half_passes(self):
        base = self.dir(self.good(), "base")
        small = self.good()
        small["land.bin"] = mapd(LINE[:6])
        self.assertEqual(vd.validate(self.dir(small, "new"), baseline=base), [])

    def test_missing_vs_baseline(self):
        base = self.dir(self.good(), "base")
        files = self.good()
        del files["labels.tsv"]
        d = self.dir(files, "new")
        self.assertProblem(vd.validate(d, baseline=base), "labels.tsv: in the baseline but missing")
        self.assertEqual(vd.validate(d, baseline=base, allow_shrink=["labels.tsv"]), [])

    def test_new_file_vs_baseline_is_fine(self):
        base = self.dir({"land.bin": mapd(LINE)}, "base")
        self.assertEqual(vd.validate(self.dir(self.good(), "new"), baseline=base), [])

    def test_main_exit_status(self):
        good = self.dir(self.good(), "good")
        self.assertEqual(vd.main([str(good)]), 0)
        files = self.good()
        files["land.bin"] = b"junk"
        self.assertEqual(vd.main([str(self.dir(files, "bad"))]), 1)


class Decoders(unittest.TestCase):
    def test_varint_limits(self):
        with self.assertRaises(vd.Bad):
            vd.Reader(b"\xff" * 10 + b"\x01").varint()
        with self.assertRaises(vd.Bad):
            vd.Reader(b"\x80").varint()
        self.assertEqual(vd.Reader(b"\xff" * 9 + b"\x01").varint(), 2**64 - 1)

    def test_huge_counts_do_not_allocate(self):
        with self.assertRaises(vd.Bad):
            vd.decode_shapes(b"MAPD\x01" + b"\xff" * 8 + b"\x01")
        with self.assertRaises(vd.Bad):
            vd.decode_countries(b"MAPC\x03\x80\x08\xff\xff\xff\xff\x0f")

    def test_country_header_rules(self):
        for data in (b"MAPC\x02\x01\x00", b"MAPC\x03\x00\x00", b"MAPC\x03\x01\xff\xff\xff\xff\x0f"):
            with self.assertRaises(vd.Bad):
                vd.decode_countries(data)
        self.assertEqual(vd.decode_countries(b"MAPC\x03\x01\x00"), 0)


if __name__ == "__main__":
    unittest.main()
