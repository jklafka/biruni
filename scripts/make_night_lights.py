#!/usr/bin/env python3
"""Build assets/data/night_lights.webp, the Night lights layer's texture.

Source: NASA Earth Observatory, "Earth at Night" (Black Marble) 2016 color
composite from Suomi NPP VIIRS, 13500x6750 JPEG (about 8.1 MB):

  https://assets.science.nasa.gov/content/dam/science/esd/eo/images/imagerecords/144000/144898/BlackMarble_2016_3km.jpg

Listing page: https://science.nasa.gov/earth/earth-observatory/earth-at-night/maps/

The URL, size and sha256 are pinned in data-sources.lock.json; the script
fetches the file through scripts/data_sources.py (cached in .data-cache/,
verified, an error on any mismatch), resamples it to 4096x2048 (Lanczos) and
writes a lossy WebP. The original is not committed. Needs Pillow built with
WebP support (`pip install Pillow`).

    python3 scripts/make_night_lights.py              # fetch the pinned file, then convert
    python3 scripts/make_night_lights.py src.jpg      # convert a local copy

The output is not byte-for-byte reproducible across Pillow/libwebp versions,
so the "Data" CI job does not regenerate it; scripts/check-data.sh only
checks that it is a 4096x2048 WebP under 2 MB.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import data_sources  # noqa: E402

SOURCE = "nasa-black-marble-2016"  # entry in data-sources.lock.json
SOURCE_SIZE = (13500, 6750)
WIDTH, HEIGHT, QUALITY = 4096, 2048, 85
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "data", "night_lights.webp")


def full_image(src=None):
    """The source at its full 13500x6750 (also used by scripts/make_tiles.py).
    Fetches the pinned source unless given a path."""
    from PIL import Image

    Image.MAX_IMAGE_PIXELS = None  # 91 MP source, from a pinned hash
    if src is None:
        try:
            src = data_sources.fetch(SOURCE)
        except (data_sources.DataSourceError, OSError) as e:
            sys.exit(f"error: {e}")
    im = Image.open(src).convert("RGB")
    if im.size != SOURCE_SIZE:
        sys.exit(f"unexpected source size {im.size}, wanted {SOURCE_SIZE}")
    return im


def main():
    from PIL import Image

    im = full_image(sys.argv[1] if len(sys.argv) > 1 else None)
    im = im.resize((WIDTH, HEIGHT), Image.LANCZOS)
    im.save(OUT, "WEBP", quality=QUALITY, method=6)
    print(f"wrote {os.path.normpath(OUT)}: {os.path.getsize(OUT)} bytes")


if __name__ == "__main__":
    main()
