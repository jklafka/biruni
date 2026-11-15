#!/usr/bin/env python3
"""Unit tests for scripts/hash_data.py (stdlib only).

Run: python3 scripts/test_hash_data.py (scripts/check-data.sh runs it too).
"""
import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("hash_data.py")


def run(site):
    return subprocess.run(
        [sys.executable, "-I", str(SCRIPT), str(site)], capture_output=True, text=True
    )


def make_site(root, files):
    data = Path(root) / "data"
    data.mkdir()
    for name, body in files.items():
        (data / name).write_bytes(body)
    return data


def short_hash(body):
    return hashlib.sha256(body).hexdigest()[:16]


class HashData(unittest.TestCase):
    def test_moves_files_and_writes_the_manifest(self):
        with tempfile.TemporaryDirectory() as root:
            data = make_site(root, {"land.bin": b"\x01\x02", "labels.tsv": b"a\tb\n"})
            result = run(root)
            self.assertEqual(result.returncode, 0, result.stderr)
            manifest = json.loads((data / "manifest.json").read_text())
            land_hash, labels_hash = short_hash(b"\x01\x02"), short_hash(b"a\tb\n")
            land = f"files/land.{land_hash}.bin"
            labels = f"files/labels.{labels_hash}.tsv"
            self.assertEqual(
                manifest, {"version": 1, "files": {"land.bin": land, "labels.tsv": labels}}
            )
            self.assertEqual((data / land).read_bytes(), b"\x01\x02")
            # Only the manifest and the hashed files remain.
            self.assertEqual(sorted(p.name for p in data.iterdir()), ["files", "manifest.json"])

    def test_output_is_deterministic(self):
        outputs = []
        for order in (["b.bin", "a.bin"], ["a.bin", "b.bin"]):
            with tempfile.TemporaryDirectory() as root:
                data = make_site(root, {name: name.encode() for name in order})
                self.assertEqual(run(root).returncode, 0)
                outputs.append((data / "manifest.json").read_bytes())
        self.assertEqual(outputs[0], outputs[1])

    def test_changed_bytes_change_the_name(self):
        names = []
        for body in (b"one", b"two"):
            with tempfile.TemporaryDirectory() as root:
                data = make_site(root, {"a.bin": body})
                run(root)
                names.append(json.loads((data / "manifest.json").read_text())["files"]["a.bin"])
        self.assertNotEqual(names[0], names[1])

    def test_no_data_files_is_an_error(self):
        with tempfile.TemporaryDirectory() as root:
            make_site(root, {})
            self.assertNotEqual(run(root).returncode, 0)

    def test_missing_data_directory_is_an_error(self):
        with tempfile.TemporaryDirectory() as root:
            self.assertNotEqual(run(root).returncode, 0)


if __name__ == "__main__":
    unittest.main()
