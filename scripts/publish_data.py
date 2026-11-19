#!/usr/bin/env python3
"""Helpers for publishing the built data files to R2 (stdlib only).

deploy.yml runs these on pushes to main, around the wrangler upload:

  fetch-baseline URL OUT   download the published manifest at URL and its
                           files into OUT, laid out like a built data dir
                           (for validate_data.py --baseline). Writes nothing
                           and succeeds if URL has no manifest yet.
  plan DIR BASE_URL        print the files under DIR/files that BASE_URL/files
                           does not have yet, one per line:
                           "<hashed name><TAB><content type>"
  rewrite DIR BASE_URL     point DIR/manifest.json at BASE_URL/files/<name>
                           (absolute https URLs) and delete DIR/files
  check-tilesets FILE      check that every tile pack in FILE (tilesets.json)
                           is served: a range request for its directory gets
                           206 with CORS, the full size and a header that
                           matches the entry (docs/tile-streaming.md).
                           Retries for about 30 s per pack.

DIR is a built data dir (dist/data): manifest.json plus files/. See
hash_data.py for the manifest format.
"""

import json
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

MANIFEST = "manifest.json"
FILES_DIR = "files"
USER_AGENT = "data-publisher"
TIMEOUT = 60

CONTENT_TYPES = {
    ".bin": "application/octet-stream",
    ".tsv": "text/plain; charset=utf-8",
    ".webp": "image/webp",
    ".json": "application/json",
}


def content_type(name):
    ext = Path(name).suffix
    try:
        return CONTENT_TYPES[ext]
    except KeyError:
        raise ValueError(f"no content type for {name!r}; add {ext!r} to CONTENT_TYPES")


def manifest_text(manifest):
    """Same canonical form as hash_data.py."""
    return json.dumps(manifest, indent=2, sort_keys=True) + "\n"


def load_manifest(data_dir):
    manifest = json.loads((Path(data_dir) / MANIFEST).read_text("utf-8"))
    if not isinstance(manifest.get("files"), dict) or not manifest["files"]:
        raise ValueError("manifest has no files")
    return manifest


def hashed_name(path):
    """The file name of a 'files/<name>' manifest path, or a ValueError."""
    prefix = FILES_DIR + "/"
    if not isinstance(path, str) or not path.startswith(prefix):
        raise ValueError(f"manifest path {path!r} is not under {prefix}")
    name = path[len(prefix):]
    if not name or "/" in name or name in (".", ".."):
        raise ValueError(f"bad manifest path {path!r}")
    return name


def check_base_url(base_url):
    parts = urllib.parse.urlsplit(base_url)
    if parts.scheme != "https" or not parts.netloc:
        raise ValueError(f"base URL {base_url!r} must be https")
    return base_url.rstrip("/")


def file_url(base_url, name):
    return f"{base_url.rstrip('/')}/{FILES_DIR}/{urllib.parse.quote(name)}"


# Sent on every request. Cloudflare's cache may keep the first response it
# sees for an R2 file, CORS headers included; a request without Origin could
# store a copy without Access-Control-Allow-Origin that browsers then get.
ORIGIN = "https://biruni.jklafka.cloud"


def request(url, method="GET"):
    headers = {"User-Agent": USER_AGENT, "Origin": ORIGIN}
    req = urllib.request.Request(url, method=method, headers=headers)
    return urllib.request.urlopen(req, timeout=TIMEOUT)


def rewrite(data_dir, base_url):
    base = check_base_url(base_url)
    data = Path(data_dir)
    manifest = load_manifest(data)
    files = {key: file_url(base, hashed_name(p)) for key, p in manifest["files"].items()}
    manifest["files"] = files
    (data / MANIFEST).write_bytes(manifest_text(manifest).encode("utf-8"))
    files_dir = data / FILES_DIR
    for path in sorted(files_dir.glob("*")):
        path.unlink()
    files_dir.rmdir()
    return files


def present(base_url, name):
    """Whether BASE_URL/files/<name> exists (HEAD 200). Other errors raise."""
    try:
        with request(file_url(base_url, name), "HEAD") as resp:
            return resp.status == 200
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return False
        raise


def plan(data_dir, base_url):
    """[(hashed name, content type)] for files BASE_URL does not have."""
    base = check_base_url(base_url)
    manifest = load_manifest(data_dir)
    names = sorted({hashed_name(p) for p in manifest["files"].values()})
    missing = []
    for name in names:
        if not (Path(data_dir) / FILES_DIR / name).is_file():
            raise ValueError(f"{name} is in the manifest but not in {data_dir}/files")
        ctype = content_type(name)
        if not present(base, name):
            missing.append((name, ctype))
    return missing


def fetch_baseline(url, out_dir):
    """Download a published manifest and its files. Returns False if none."""
    try:
        with request(url) as resp:
            manifest = json.loads(resp.read())
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return False
        raise
    if not isinstance(manifest.get("files"), dict) or not manifest["files"]:
        raise ValueError(f"{url} is not a data manifest")
    files = {}
    downloads = {}
    for key, path in manifest["files"].items():
        # Relative to the manifest, or an absolute URL (published to R2).
        full = urllib.parse.urljoin(url, path)
        name = Path(urllib.parse.urlsplit(full).path).name
        if not name:
            raise ValueError(f"bad manifest path {path!r}")
        files[key] = f"{FILES_DIR}/{name}"
        downloads[name] = full
    out = Path(out_dir)
    (out / FILES_DIR).mkdir(parents=True, exist_ok=True)
    for name, full in sorted(downloads.items()):
        with request(full) as resp:
            (out / FILES_DIR / name).write_bytes(resp.read())
    manifest["files"] = files
    (out / MANIFEST).write_bytes(manifest_text(manifest).encode("utf-8"))
    return True


def check_pack_response(entry, status, headers, body):
    """Problems with the answer to a directory range request for a tileset
    `entry`; headers is a {lowercase name: value} dict."""
    import tile_pack

    problems = []
    if status != 206:
        problems.append(f"status {status}, want 206 (range support)")
    if headers.get("access-control-allow-origin") != ORIGIN:
        problems.append(f"no Access-Control-Allow-Origin for {ORIGIN}")
    m = re.fullmatch(r"bytes \d+-\d+/(\d+)", headers.get("content-range", ""))
    if not m or int(m.group(1)) != entry["size"]:
        problems.append(f"Content-Range {headers.get('content-range')!r}, want a total of {entry['size']}")
    try:
        info, _ = tile_pack.read_header(body)
        if (info["min_level"], info["max_level"]) != (entry["min_level"], entry["max_level"]):
            problems.append(f"levels {info['min_level']}..{info['max_level']} do not match the entry")
    except tile_pack.PackError as e:
        problems.append(f"bad pack header: {e}")
    return problems


def check_tilesets(path, tries=6, delay=5):
    """Raise ValueError unless every pack in tilesets.json is served."""
    import tile_pack

    doc = json.loads(Path(path).read_text("utf-8"))
    for entry in doc["tilesets"]:
        end = tile_pack.header_len(entry["min_level"], entry["max_level"]) - 1
        problems = []
        for attempt in range(tries):
            if attempt:
                time.sleep(delay)
            req = urllib.request.Request(entry["url"], headers={
                "User-Agent": USER_AGENT, "Origin": ORIGIN, "Range": f"bytes=0-{end}"})
            try:
                with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
                    headers = {k.lower(): v for k, v in resp.headers.items()}
                    problems = check_pack_response(entry, resp.status, headers, resp.read(end + 1))
            except urllib.error.HTTPError as e:
                problems = [f"HTTP {e.code}"]
            if not problems:
                break
        if problems:
            raise ValueError(f"{entry['url']}: " + "; ".join(problems))
        print(f"{entry['url']}: served")
    return len(doc["tilesets"])


def main(argv):
    try:
        if len(argv) == 4 and argv[1] == "fetch-baseline":
            if fetch_baseline(argv[2], argv[3]):
                print(f"baseline from {argv[2]} saved to {argv[3]}")
            else:
                print(f"notice: {argv[2]} has no manifest; no baseline")
        elif len(argv) == 4 and argv[1] == "plan":
            for name, ctype in plan(argv[2], argv[3]):
                print(f"{name}\t{ctype}")
        elif len(argv) == 3 and argv[1] == "check-tilesets":
            n = check_tilesets(argv[2])
            print(f"{n} tile packs served")
        elif len(argv) == 4 and argv[1] == "rewrite":
            files = rewrite(argv[2], argv[3])
            print(f"manifest now points {len(files)} files at {argv[3]}")
        else:
            print(__doc__, file=sys.stderr)
            return 2
    except (ValueError, OSError, urllib.error.URLError) as e:
        print(f"publish_data.py: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
