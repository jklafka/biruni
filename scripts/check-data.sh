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
# Hand-processed from NASA downloads (scripts/make_blue_marble.py); CI cannot
# fetch them, so they are checked for shape only (below), not regenerated.
external=(blue_marble.webp)
is_external() { local x; for x in "${external[@]}"; do [ "$x" = "$1" ] && return 0; done; return 1; }

for f in assets/data/*; do
  name="$(basename "$f")"
  is_external "$name" && continue
  [ -e "$tmp/$name" ] || drift+=("$name (committed but no longer generated)")
done

# The imagery must be a 4096x2048 WebP under 2 MB (what the renderer expects).
img=assets/data/blue_marble.webp
if [ ! -f "$img" ]; then
  drift+=("blue_marble.webp (missing; see scripts/make_blue_marble.py)")
else
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
  [ "$dims" = "4096 2048" ] || drift+=("blue_marble.webp is not a lossy 4096x2048 WebP (got: $dims)")
  [ "$size" -lt 2097152 ] || drift+=("blue_marble.webp is $size bytes, over the 2 MB budget")
fi

if [ ${#drift[@]} -gt 0 ]; then
  echo "error: assets/data/ is out of date with data-src/ and scripts/convert_geojson.py:" >&2
  printf '  - %s\n' "${drift[@]}" >&2
  echo "Regenerate and commit with: python3 scripts/convert_geojson.py" >&2
  exit 1
fi
echo "assets/data/ matches the generated output."
