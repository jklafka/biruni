#!/usr/bin/env python3
"""Build assets/data/blue_marble.webp, the imagery layer's texture.

Source: NASA Earth Observatory, "Blue Marble: Next Generation w/ Topography
and Bathymetry", July 2004, 5400x2700 JPEG (about 2.3 MB):

  https://assets.science.nasa.gov/content/dam/science/esd/eo/images/bmng/bmng-topography-bathymetry/july/world.topo.bathy.200407.3x5400x2700.jpg

Listing page: https://science.nasa.gov/earth/earth-observatory/blue-marble-next-generation/base-topography-bathymetry

The URL, size and sha256 are pinned in data-sources.lock.json; the script
fetches the file through scripts/data_sources.py (cached in .data-cache/,
verified, an error on any mismatch), resamples it to 4096x2048 (Lanczos) and
writes a lossy WebP. The original is not committed. Needs Pillow built with
WebP support (`pip install Pillow`).

    python3 scripts/make_blue_marble.py              # fetch the pinned file, then convert
    python3 scripts/make_blue_marble.py src.jpg      # convert a local copy

The output is not byte-for-byte reproducible across Pillow/libwebp versions,
so the "Data" CI job does not regenerate it (CI cannot download from NASA);
scripts/check-data.sh only checks that it is a 4096x2048 WebP under 2 MB.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import data_sources  # noqa: E402

SOURCE = "nasa-blue-marble-200407"  # entry in data-sources.lock.json
WIDTH, HEIGHT, QUALITY = 4096, 2048, 85
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "data", "blue_marble.webp")


def main():
    from PIL import Image

    if len(sys.argv) > 1:
        src = sys.argv[1]
    else:
        try:
            src = data_sources.fetch(SOURCE)
        except (data_sources.DataSourceError, OSError) as e:
            sys.exit(f"error: {e}")
    im = Image.open(src).convert("RGB")
    if im.size != (5400, 2700):
        sys.exit(f"unexpected source size {im.size}, wanted 5400x2700")
    im = im.resize((WIDTH, HEIGHT), Image.LANCZOS)
    im.save(OUT, "WEBP", quality=QUALITY, method=6)
    print(f"wrote {os.path.normpath(OUT)}: {os.path.getsize(OUT)} bytes")


if __name__ == "__main__":
    main()
