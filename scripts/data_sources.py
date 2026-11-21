#!/usr/bin/env python3
"""Lockfile for upstream data sources: data-sources.lock.json.

Git records WHICH bytes a build uses (URL, version, sha256, size); the bytes
of large sources live outside git. Fetching verifies every file against the
lockfile and fails on any mismatch. A refresh is a PR that runs `bump`.

    python3 scripts/data_sources.py fetch NAME... [--cache DIR]
    python3 scripts/data_sources.py bump NAME [--url URL] [--version V]
    python3 scripts/data_sources.py check

`check` uses no network. The `license` of each entry is a key that appears as
`[key: <id>]` in a dataset row of DATA_LICENSES.md.
"""
import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LOCKFILE = os.path.join(ROOT, "data-sources.lock.json")
LICENSES = os.path.join(ROOT, "DATA_LICENSES.md")
DEFAULT_CACHE = os.path.join(ROOT, ".data-cache")
LOCK_VERSION = 1
FIELDS = ("name", "url", "version", "sha256", "size", "license", "notes")
REQUIRED = ("name", "url", "version", "sha256", "size", "license")
CHUNK = 1 << 20
USER_AGENT = "data-sources-fetch"


class DataSourceError(Exception):
    pass


def load_lock(path=LOCKFILE):
    try:
        with open(path, "rb") as f:
            return json.loads(f.read().decode("utf-8"))
    except (OSError, ValueError) as e:
        raise DataSourceError(f"cannot read {path}: {e}")


def dump_lock(doc):
    """Canonical text: fixed key order, two-space indent, trailing newline."""
    out = {
        "version": doc["version"],
        "sources": [{k: s[k] for k in FIELDS if k in s} for s in doc["sources"]],
    }
    return (json.dumps(out, indent=2, ensure_ascii=False) + "\n").encode("utf-8")


def license_keys(path=LICENSES):
    with open(path, "rb") as f:
        text = f.read().decode("utf-8")
    return set(re.findall(r"\[key: ([a-z0-9][a-z0-9-]*)\]", text))


def validate(doc, keys):
    """Return a list of problems (empty when the lockfile is fine)."""
    errs = []
    if not isinstance(doc, dict) or doc.get("version") != LOCK_VERSION:
        return [f"top level must be an object with version {LOCK_VERSION}"]
    sources = doc.get("sources")
    if not isinstance(sources, list):
        return ["`sources` must be a list"]
    names = []
    for i, s in enumerate(sources):
        if not isinstance(s, dict):
            errs.append(f"sources[{i}] is not an object")
            continue
        label = s.get("name") if isinstance(s.get("name"), str) else f"sources[{i}]"
        for k in REQUIRED:
            if k not in s:
                errs.append(f"{label}: missing `{k}`")
        for k in s:
            if k not in FIELDS:
                errs.append(f"{label}: unknown field `{k}`")
        for k in ("name", "url", "version", "sha256", "license", "notes"):
            if k in s and not isinstance(s[k], str):
                errs.append(f"{label}: `{k}` must be a string")
        if isinstance(s.get("name"), str):
            names.append(s["name"])
            if not re.fullmatch(r"[a-z0-9][a-z0-9._-]*", s["name"]):
                errs.append(f"{label}: name must match [a-z0-9][a-z0-9._-]*")
        sha = s.get("sha256")
        if isinstance(sha, str) and not re.fullmatch(r"[0-9a-f]{64}", sha):
            errs.append(f"{label}: sha256 must be 64 lowercase hex digits")
        size = s.get("size")
        if "size" in s and (type(size) is not int or size <= 0):
            errs.append(f"{label}: size must be a positive integer")
        url = s.get("url")
        if isinstance(url, str) and not url.startswith("https://"):
            errs.append(f"{label}: url must be https")
        lic = s.get("license")
        if isinstance(lic, str) and lic not in keys:
            errs.append(f"{label}: license key `{lic}` not found in DATA_LICENSES.md")
    if names != sorted(names):
        errs.append("sources are not sorted by name")
    if len(set(names)) != len(names):
        errs.append("duplicate source names")
    return errs


def find(doc, name):
    for s in doc["sources"]:
        if s.get("name") == name:
            return s
    raise DataSourceError(f"no source named {name!r} in the lockfile")


def verify(path, sha256, size):
    """Raise DataSourceError unless the file has exactly this size and hash."""
    actual_size = os.path.getsize(path)
    if actual_size != size:
        raise DataSourceError(f"size mismatch for {path}: expected {size}, got {actual_size}")
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            chunk = f.read(CHUNK)
            if not chunk:
                break
            h.update(chunk)
    if h.hexdigest() != sha256:
        raise DataSourceError(f"sha256 mismatch for {path}: expected {sha256}, got {h.hexdigest()}")


def download(url, dest_dir):
    """Download url to a new temp file in dest_dir; return (path, sha256, size)."""
    os.makedirs(dest_dir, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=dest_dir, suffix=".part")
    h = hashlib.sha256()
    size = 0
    try:
        # Some hosts (Wikimedia) refuse urllib's default User-Agent.
        req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
        with os.fdopen(fd, "wb") as out, urllib.request.urlopen(req, timeout=120) as resp:
            while True:
                chunk = resp.read(CHUNK)
                if not chunk:
                    break
                out.write(chunk)
                h.update(chunk)
                size += len(chunk)
    except BaseException:
        os.unlink(tmp)
        raise
    return tmp, h.hexdigest(), size


def cache_path(entry, cache):
    # Keyed by hash, so a bump never serves stale bytes under an old name.
    ext = os.path.splitext(entry["url"].split("?")[0])[1]
    return os.path.join(cache, f"{entry['name']}-{entry['sha256'][:16]}{ext}")


def fetch_entry(entry, cache=DEFAULT_CACHE):
    """Return the path of a verified copy of the source, downloading if needed."""
    dest = cache_path(entry, cache)
    if os.path.exists(dest):
        try:
            verify(dest, entry["sha256"], entry["size"])
            return dest
        except DataSourceError:
            os.unlink(dest)
    print(f"downloading {entry['url']}", file=sys.stderr)
    tmp, _, _ = download(entry["url"], cache)
    try:
        verify(tmp, entry["sha256"], entry["size"])
    except DataSourceError:
        os.unlink(tmp)
        raise
    os.replace(tmp, dest)
    return dest


def fetch(name, cache=DEFAULT_CACHE, lock=LOCKFILE):
    return fetch_entry(find(load_lock(lock), name), cache)


def bump(name, url=None, version=None, lock=LOCKFILE, cache=DEFAULT_CACHE):
    doc = load_lock(lock)
    entry = find(doc, name)
    if url:
        entry["url"] = url
    if version:
        entry["version"] = version
    tmp, sha, size = download(entry["url"], cache)
    os.unlink(tmp)
    if size == 0:
        raise DataSourceError("downloaded file is empty")
    entry["sha256"], entry["size"] = sha, size
    with open(lock, "wb") as f:
        f.write(dump_lock(doc))
    return entry


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    pf = sub.add_parser("fetch")
    pf.add_argument("names", nargs="+")
    pf.add_argument("--cache", default=DEFAULT_CACHE)
    pb = sub.add_parser("bump")
    pb.add_argument("name")
    pb.add_argument("--url")
    pb.add_argument("--version")
    pb.add_argument("--cache", default=DEFAULT_CACHE)
    sub.add_parser("check")
    a = p.parse_args(argv)
    try:
        if a.cmd == "fetch":
            for n in a.names:
                print(fetch(n, a.cache))
        elif a.cmd == "bump":
            e = bump(a.name, a.url, a.version, cache=a.cache)
            print(f"{e['name']}: sha256 {e['sha256']} size {e['size']}")
        else:
            errs = validate(load_lock(), license_keys())
            if errs:
                for e in errs:
                    print(f"error: {e}", file=sys.stderr)
                return 1
            print("data-sources.lock.json is valid.")
    except (DataSourceError, OSError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
