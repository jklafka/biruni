#!/usr/bin/env python3
"""Unit tests for scripts/data_sources.py (stdlib only, no network).

Run: python3 scripts/test_data_sources.py (scripts/check-data.sh runs it too).
Downloads use file:// URLs; the https rule is checked by validate() only.
"""
import copy
import hashlib
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import data_sources as ds  # noqa: E402

PAYLOAD = b"hello biruni\n"
KEYS = {"test-license"}


def entry(**kw):
    e = {
        "name": "a",
        "url": "https://example.com/a.bin",
        "version": "1",
        "sha256": hashlib.sha256(PAYLOAD).hexdigest(),
        "size": len(PAYLOAD),
        "license": "test-license",
    }
    e.update(kw)
    return e


def doc(*sources):
    return {"version": 1, "sources": list(sources)}


class Base(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.dir = Path(self._tmp.name)
        self.src = self.dir / "payload.bin"
        self.src.write_bytes(PAYLOAD)
        self.cache = self.dir / "cache"

    def file_entry(self, **kw):
        return entry(url=self.src.as_uri(), **kw)


class VerifyTests(Base):
    def test_ok(self):
        ds.verify(self.src, entry()["sha256"], len(PAYLOAD))

    def test_hash_mismatch(self):
        with self.assertRaisesRegex(ds.DataSourceError, "sha256 mismatch"):
            ds.verify(self.src, "0" * 64, len(PAYLOAD))

    def test_size_mismatch(self):
        with self.assertRaisesRegex(ds.DataSourceError, "size mismatch"):
            ds.verify(self.src, entry()["sha256"], len(PAYLOAD) + 1)


class FetchTests(Base):
    def test_fetch_and_cache_hit(self):
        e = self.file_entry()
        path = ds.fetch_entry(e, str(self.cache))
        self.assertEqual(Path(path).read_bytes(), PAYLOAD)
        # Remove the upstream: a second fetch must be served from the cache.
        self.src.unlink()
        self.assertEqual(ds.fetch_entry(e, str(self.cache)), path)
        self.assertEqual([p for p in self.cache.iterdir() if p.suffix == ".part"], [])

    def test_mismatch_fails_and_leaves_no_file(self):
        e = self.file_entry(sha256="0" * 64)
        with self.assertRaises(ds.DataSourceError):
            ds.fetch_entry(e, str(self.cache))
        self.assertEqual(list(self.cache.iterdir()), [])

    def test_size_mismatch_fails(self):
        e = self.file_entry(size=len(PAYLOAD) + 1)
        with self.assertRaisesRegex(ds.DataSourceError, "size mismatch"):
            ds.fetch_entry(e, str(self.cache))

    def test_corrupt_cache_is_replaced(self):
        e = self.file_entry()
        path = Path(ds.fetch_entry(e, str(self.cache)))
        path.write_bytes(b"corrupt")
        ds.fetch_entry(e, str(self.cache))
        self.assertEqual(path.read_bytes(), PAYLOAD)

    def test_bump_rewrites_entry(self):
        lock = self.dir / "lock.json"
        lock.write_bytes(ds.dump_lock(doc(self.file_entry(sha256="0" * 64, size=1))))
        ds.bump("a", version="2", lock=str(lock), cache=str(self.cache))
        e = json.loads(lock.read_text())["sources"][0]
        self.assertEqual((e["version"], e["size"]), ("2", len(PAYLOAD)))
        self.assertEqual(e["sha256"], hashlib.sha256(PAYLOAD).hexdigest())
        self.assertEqual(lock.read_bytes(), ds.dump_lock(json.loads(lock.read_text())))


class ValidateTests(unittest.TestCase):
    def errs(self, d):
        return ds.validate(d, KEYS)

    def test_valid(self):
        self.assertEqual(self.errs(doc(entry(name="a"), entry(name="b", notes="x"))), [])

    def test_malformed(self):
        self.assertTrue(self.errs([]))
        self.assertTrue(self.errs({"version": 2, "sources": []}))
        self.assertTrue(self.errs({"version": 1, "sources": {}}))
        self.assertTrue(self.errs(doc("nope")))

    def test_malformed_file(self):
        with tempfile.TemporaryDirectory() as t:
            p = Path(t) / "l.json"
            p.write_text("{not json")
            with self.assertRaises(ds.DataSourceError):
                ds.load_lock(str(p))

    def test_missing_and_unknown_field(self):
        e = entry()
        del e["url"]
        self.assertTrue(any("missing `url`" in m for m in self.errs(doc(e))))
        self.assertTrue(any("unknown field" in m for m in self.errs(doc(entry(extra=1)))))

    def test_unknown_license(self):
        self.assertTrue(any("license key" in m for m in self.errs(doc(entry(license="nope")))))

    def test_unsorted_and_duplicate(self):
        self.assertTrue(any("sorted" in m for m in self.errs(doc(entry(name="b"), entry(name="a")))))
        self.assertTrue(any("duplicate" in m for m in self.errs(doc(entry(name="a"), entry(name="a")))))

    def test_non_https(self):
        self.assertTrue(any("https" in m for m in self.errs(doc(entry(url="http://x/a")))))
        self.assertTrue(any("https" in m for m in self.errs(doc(entry(url="file:///a")))))

    def test_bad_hash_and_size(self):
        for bad in ("abc", "G" * 64, "A" * 64):
            self.assertTrue(self.errs(doc(entry(sha256=bad))), bad)
        for bad in (0, -1, "5", True, 1.5):
            self.assertTrue(self.errs(doc(entry(size=bad))), bad)


class RepoTests(unittest.TestCase):
    def test_committed_lockfile_is_valid_and_canonical(self):
        d = ds.load_lock()
        self.assertEqual(ds.validate(d, ds.license_keys()), [])
        self.assertEqual(Path(ds.LOCKFILE).read_bytes(), ds.dump_lock(copy.deepcopy(d)))


if __name__ == "__main__":
    unittest.main()
