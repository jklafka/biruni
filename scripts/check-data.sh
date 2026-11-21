#!/usr/bin/env bash
# Check that assets/data/ is exactly what scripts/convert_geojson.py generates
# from data-src/. Run from anywhere; CI runs it too (job "Data").
set -euo pipefail
cd "$(dirname "$0")/.."

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

python3 scripts/convert_geojson.py data-src "$tmp" > /dev/null
python3 scripts/test_frame_box.py > /dev/null 2>&1 || { python3 scripts/test_frame_box.py; exit 1; }
python3 scripts/test_hash_data.py > /dev/null 2>&1 || { python3 scripts/test_hash_data.py; exit 1; }
python3 scripts/test_data_sources.py > /dev/null 2>&1 || { python3 scripts/test_data_sources.py; exit 1; }
python3 scripts/test_validate_data.py > /dev/null 2>&1 || { python3 scripts/test_validate_data.py; exit 1; }
python3 scripts/test_publish_data.py > /dev/null 2>&1 || { python3 scripts/test_publish_data.py; exit 1; }
python3 scripts/test_tile_pack.py > /dev/null 2>&1 || { python3 scripts/test_tile_pack.py; exit 1; }
python3 scripts/test_convert_stars.py > /dev/null 2>&1 || { python3 scripts/test_convert_stars.py; exit 1; }
python3 scripts/test_convert_constellations.py > /dev/null 2>&1 || { python3 scripts/test_convert_constellations.py; exit 1; }
python3 scripts/test_fetch_space.py > /dev/null 2>&1 || { python3 scripts/test_fetch_space.py; exit 1; }
# Lockfile schema and license keys only; no network.
python3 scripts/data_sources.py check > /dev/null || { python3 scripts/data_sources.py check; exit 1; }

drift=()
for f in "$tmp"/*; do
  name="$(basename "$f")"
  if [ ! -f "assets/data/$name" ]; then
    drift+=("$name (generated but not committed)")
  elif ! cmp -s "$f" "assets/data/$name"; then
    drift+=("$name (differs)")
  fi
done
# Hand-processed from pinned downloads (scripts/make_blue_marble.py,
# scripts/make_terrain.py, scripts/make_night_lights.py, scripts/make_milky_way.py);
# CI cannot fetch them, so they are checked for shape only (below), not regenerated.
external=(blue_marble.webp night_lights.webp terrain.webp milky_way.webp
  planet_earth.webp planet_moon.webp planet_mars.webp planet_mercury.webp planet_jupiter.webp)
is_external() { local x; for x in "${external[@]}"; do [ "$x" = "$1" ] && return 0; done; return 1; }

# The tile pack list (docs/tile-streaming.md): written by scripts/make_tiles.py
# when packs are published to R2, so schema-checked only.
python3 -c 'import sys; sys.path.insert(0, "scripts"); import validate_data as v
try:
    v.check_tilesets(open("assets/data/tilesets.json", "rb").read())
except v.Bad as e:
    sys.exit(f"assets/data/tilesets.json: {e}")' || drift+=("tilesets.json (invalid, see above)")

for f in assets/data/*; do
  name="$(basename "$f")"
  is_external "$name" && continue
  [ "$name" = tilesets.json ] && continue
  [ "$name" = stars.bin ] && continue
  [ "$name" = constellations.bin ] && continue
  [ "$name" = space.bin ] && continue
  [ -e "$tmp/$name" ] || drift+=("$name (committed but no longer generated)")
done

# Each image layer must be a 4096x2048 WebP under 2 MB (what the renderer
# expects); the sky's Milky Way and the planet maps are 2048x1024.
for name in "${external[@]}"; do
  img="assets/data/$name"
  if [ ! -f "$img" ]; then
    drift+=("$name (missing; see the make_*.py script that builds it)")
    continue
  fi
  size="$(wc -c < "$img" | tr -d ' ')"
  # RIFF....WEBPVP8 , then for lossy: canvas size at bytes 26..29 (14-bit LE).
  dims="$(python3 - "$img" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read(40)
ok = d[:4] == b"RIFF" and d[8:12] == b"WEBP" and d[12:16] == b"VP8 "
w, h = struct.unpack("<HH", d[26:30]) if ok else (0, 0)
print(w & 0x3FFF, h & 0x3FFF)
PY
)"
  want="4096 2048"
  case "$name" in milky_way.webp | planet_*.webp) want="2048 1024" ;; esac
  [ "$dims" = "$want" ] || drift+=("$name is not a lossy ${want/ /x} WebP (got: $dims)")
  [ "$size" -lt 2097152 ] || drift+=("$name is $size bytes, over the 2 MB budget")
done

# The star catalog (scripts/convert_stars.py) comes from the pinned HYG
# download, which CI does not fetch: check that it decodes and looks sane.
python3 - <<'PY' || drift+=("stars.bin (see above; rebuild with scripts/convert_stars.py)")
import sys
sys.path.insert(0, "scripts")
import convert_stars as cs
data = open("assets/data/stars.bin", "rb").read()
stars = cs.decode(data)
names = sum(1 for s in stars if s[4])
problems = []
if not 30000 <= len(stars) <= 60000:
    problems.append(f"{len(stars)} stars")
if names < 300:
    problems.append(f"only {names} names")
if len(data) > 1 << 20:
    problems.append(f"{len(data)} bytes, over 1 MB")
if any(s[2] > cs.MAG_LIMIT + 0.05 for s in stars):
    problems.append("stars fainter than the limit")
if problems:
    sys.exit("assets/data/stars.bin: " + ", ".join(problems))
PY

# Spacecraft and small bodies (scripts/fetch_space.py): a snapshot from JPL
# that deploys replace with fresh data. CI does not ask JPL; check that the
# snapshot decodes and looks sane.
python3 scripts/fetch_space.py --check assets/data/space.bin || drift+=("space.bin (see above; refetch with scripts/fetch_space.py)")

# Constellation figures (scripts/convert_constellations.py), from pinned
# Stellarium and HYG downloads: all 88, decodable.
python3 - <<'PY' || drift+=("constellations.bin (see above; rebuild with scripts/convert_constellations.py)")
import sys
sys.path.insert(0, "scripts")
import convert_constellations as cc
figs = cc.decode(open("assets/data/constellations.bin", "rb").read())
if len(figs) != 88:
    sys.exit(f"assets/data/constellations.bin: {len(figs)} constellations, expected 88")
PY

if [ ${#drift[@]} -gt 0 ]; then
  echo "error: assets/data/ is out of date with data-src/ and scripts/convert_geojson.py:" >&2
  printf '  - %s\n' "${drift[@]}" >&2
  echo "Regenerate and commit with: python3 scripts/convert_geojson.py" >&2
  exit 1
fi
echo "assets/data/ matches the generated output."
