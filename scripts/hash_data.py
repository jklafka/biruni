#!/usr/bin/env python3
"""Content-hash the data files of a built site and write data/manifest.json.

Runs as a Trunk post-build hook (see Trunk.toml), on the staging directory
that Trunk then moves to dist/. Every file Trunk copied from assets/data/
moves to data/files/<stem>.<hash><ext>, and data/manifest.json maps each
original name to its new path, relative to the manifest:

    {"files": {"labels.tsv": "files/labels.0123456789abcdef.tsv"}, "version": 1}

The app reads the manifest (src/data.rs), so the hashed files can be cached
as immutable (assets/_headers). The output depends only on the file bytes.

Usage: TRUNK_STAGING_DIR=<site> scripts/hash_data.py, or hash_data.py <site>.
"""

import hashlib
import json
import os
import sys
from pathlib import Path

MANIFEST = "manifest.json"
FILES_DIR = "files"
VERSION = 1
HASH_LEN = 16  # hex digits of SHA-256


def main() -> int:
    if len(sys.argv) > 2:
        print(__doc__, file=sys.stderr)
        return 2
    site = sys.argv[1] if len(sys.argv) == 2 else os.environ.get("TRUNK_STAGING_DIR")
    if not site:
        print("hash_data.py: no site directory (TRUNK_STAGING_DIR is unset)", file=sys.stderr)
        return 2
    data = Path(site) / "data"
    if not data.is_dir():
        print(f"hash_data.py: {data} is not a directory", file=sys.stderr)
        return 1

    files = data / FILES_DIR
    files.mkdir(exist_ok=True)
    entries = {}
    for path in sorted(data.iterdir()):
        if path.name == MANIFEST or not path.is_file():
            continue
        digest = hashlib.sha256(path.read_bytes()).hexdigest()[:HASH_LEN]
        hashed = f"{path.stem}.{digest}{path.suffix}"
        path.replace(files / hashed)
        entries[path.name] = f"{FILES_DIR}/{hashed}"
    if not entries:
        print(f"hash_data.py: no data files in {data}", file=sys.stderr)
        return 1

    manifest = {"version": VERSION, "files": entries}
    text = json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    (data / MANIFEST).write_bytes(text.encode("utf-8"))
    print(f"hash_data.py: {len(entries)} data files hashed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
