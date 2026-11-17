#!/usr/bin/env python3
"""Unit tests for scripts/publish_data.py (stdlib only, local server only).

Run: python3 scripts/test_publish_data.py (scripts/check-data.sh runs it too).
"""
import http.server
import json
import sys
import tempfile
import threading
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import publish_data as pd  # noqa: E402

BASE = "https://data.example.com"


def make_dir(data, files, manifest_paths=None):
    """A built data dir: files maps hashed name -> bytes, one key each."""
    data = Path(data)
    (data / "files").mkdir(parents=True)
    entries = {}
    for name, body in files.items():
        (data / "files" / name).write_bytes(body)
        key = name.split(".")[0] + "." + name.rsplit(".", 1)[1]
        entries[key] = (manifest_paths or {}).get(name, f"files/{name}")
    (data / "manifest.json").write_text(pd.manifest_text({"version": 1, "files": entries}))
    return data


class Server:
    """Serves `routes` (path -> bytes) on 127.0.0.1; 404 for the rest."""

    def __init__(self, routes):
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                body = routes.get(self.path)
                self.send_response(200 if body is not None else 404)
                self.end_headers()
                if body is not None and self.command == "GET":
                    self.wfile.write(body)

            do_HEAD = do_GET

            def log_message(self, *args):
                pass

        self.httpd = http.server.HTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.httpd.server_port}"
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.httpd.shutdown()
        self.httpd.server_close()


class Rewrite(unittest.TestCase):
    def test_absolute_urls_and_files_removed(self):
        with tempfile.TemporaryDirectory() as root:
            data = make_dir(Path(root) / "data", {"a.0123.bin": b"1", "b.4567.tsv": b"2"})
            pd.rewrite(data, BASE + "/")
            text = (data / "manifest.json").read_text()
            self.assertEqual(
                json.loads(text),
                {
                    "version": 1,
                    "files": {
                        "a.bin": f"{BASE}/files/a.0123.bin",
                        "b.tsv": f"{BASE}/files/b.4567.tsv",
                    },
                },
            )
            self.assertEqual(text, json.dumps(json.loads(text), indent=2, sort_keys=True) + "\n")
            self.assertFalse((data / "files").exists())

    def test_refuses_non_https(self):
        with tempfile.TemporaryDirectory() as root:
            data = make_dir(Path(root) / "data", {"a.0123.bin": b"1"})
            for bad in ("http://x.example.com", "x.example.com", "ftp://x"):
                with self.assertRaises(ValueError):
                    pd.rewrite(data, bad)
            self.assertTrue((data / "files").exists())

    def test_refuses_paths_outside_files(self):
        bad_paths = [
            "../a.bin",
            "other/a.0123.bin",
            "https://x.example.com/files/a.bin",
            "files/",
            "files/x/y",
        ]
        for bad in bad_paths:
            with tempfile.TemporaryDirectory() as root:
                data = make_dir(Path(root) / "data", {"a.0123.bin": b"1"}, {"a.0123.bin": bad})
                before = (data / "manifest.json").read_bytes()
                with self.assertRaises(ValueError, msg=bad):
                    pd.rewrite(data, BASE)
                self.assertEqual((data / "manifest.json").read_bytes(), before)
                self.assertTrue((data / "files" / "a.0123.bin").exists())


class ContentType(unittest.TestCase):
    def test_known_and_unknown(self):
        self.assertEqual(pd.content_type("land.0123.bin"), "application/octet-stream")
        self.assertEqual(pd.content_type("l.0123.tsv"), "text/plain; charset=utf-8")
        self.assertEqual(pd.content_type("m.0123.webp"), "image/webp")
        self.assertEqual(pd.content_type("m.0123.json"), "application/json")
        with self.assertRaises(ValueError):
            pd.content_type("x.0123.png")


class Plan(unittest.TestCase):
    def test_present_uses_head(self):
        with Server({"/files/a.0123.bin": b"1"}) as srv:
            self.assertTrue(pd.present(srv.url, "a.0123.bin"))
            self.assertFalse(pd.present(srv.url, "b.4567.tsv"))

    def test_plan_requires_https(self):
        with tempfile.TemporaryDirectory() as root:
            data = make_dir(Path(root) / "data", {"a.0123.bin": b"1"})
            with self.assertRaises(ValueError):
                pd.plan(data, "http://127.0.0.1:1")

    def test_plan_lists_only_missing_files(self):
        with tempfile.TemporaryDirectory() as root:
            data = make_dir(Path(root) / "data", {"a.0123.bin": b"1", "b.4567.tsv": b"2"})
            orig = pd.present
            pd.present = lambda base, name: name == "a.0123.bin"
            try:
                self.assertEqual(
                    pd.plan(data, BASE), [("b.4567.tsv", "text/plain; charset=utf-8")]
                )
            finally:
                pd.present = orig

    def test_unknown_extension_and_missing_file_are_errors(self):
        with tempfile.TemporaryDirectory() as root:
            data = make_dir(Path(root) / "data", {"a.0123.png": b"1"})
            with self.assertRaises(ValueError):
                pd.plan(data, BASE)
        with tempfile.TemporaryDirectory() as root:
            data = make_dir(Path(root) / "data", {"a.0123.bin": b"1"})
            (data / "files" / "a.0123.bin").unlink()
            with self.assertRaises(ValueError):
                pd.plan(data, BASE)


class FetchBaseline(unittest.TestCase):
    def test_relative_manifest(self):
        routes = {
            "/data/manifest.json": pd.manifest_text(
                {"version": 1, "files": {"a.bin": "files/a.0123.bin"}}
            ).encode(),
            "/data/files/a.0123.bin": b"AAA",
        }
        with tempfile.TemporaryDirectory() as root, Server(routes) as srv:
            out = Path(root) / "base"
            self.assertTrue(pd.fetch_baseline(srv.url + "/data/manifest.json", out))
            self.assertEqual((out / "files" / "a.0123.bin").read_bytes(), b"AAA")
            self.assertEqual(pd.load_manifest(out)["files"], {"a.bin": "files/a.0123.bin"})

    def test_absolute_manifest_maps_back_to_files(self):
        routes = {"/r2/files/a.0123.bin": b"AAA"}
        with tempfile.TemporaryDirectory() as root, Server(routes) as srv:
            manifest = {"version": 1, "files": {"a.bin": f"{srv.url}/r2/files/a.0123.bin"}}
            routes["/manifest.json"] = json.dumps(manifest).encode()
            out = Path(root) / "base"
            self.assertTrue(pd.fetch_baseline(srv.url + "/manifest.json", out))
            self.assertEqual((out / "files" / "a.0123.bin").read_bytes(), b"AAA")
            self.assertEqual(pd.load_manifest(out)["files"], {"a.bin": "files/a.0123.bin"})

    def test_missing_manifest_writes_nothing(self):
        with tempfile.TemporaryDirectory() as root, Server({}) as srv:
            out = Path(root) / "base"
            self.assertFalse(pd.fetch_baseline(srv.url + "/data/manifest.json", out))
            self.assertFalse(out.exists())

    def test_missing_file_is_an_error(self):
        routes = {
            "/m.json": json.dumps({"version": 1, "files": {"a.bin": "files/a.0123.bin"}}).encode()
        }
        with tempfile.TemporaryDirectory() as root, Server(routes) as srv:
            with self.assertRaises(Exception):
                pd.fetch_baseline(srv.url + "/m.json", Path(root) / "base")


class CorsConfig(unittest.TestCase):
    def test_rules(self):
        cfg = json.loads((Path(__file__).parent / "r2-cors.json").read_text())
        (rule,) = cfg["rules"]
        self.assertEqual(rule["allowed"]["methods"], ["GET", "HEAD"])
        self.assertIn("https://biruni.jklafka.cloud", rule["allowed"]["origins"])
        self.assertNotIn("*", rule["allowed"]["origins"])
        self.assertEqual(rule["maxAgeSeconds"], 86400)


if __name__ == "__main__":
    unittest.main()
