#!/usr/bin/env python3
"""Build assets/data/planet_<body>.webp, the solar system view's surface maps.

Every map is equirectangular in the body's planetocentric coordinates, east
longitude -180 at the left edge to +180 at the right (0 at the center), north
up: u = 0.5 + lon / 360, v = 0.5 - lat / 180, which src/shaders/body.frag
samples. Each is resampled to 2048x1024 (Lanczos) and written as a lossy
WebP. Sources, pinned in data-sources.lock.json and fetched through
scripts/data_sources.py (all US government works):

    earth    nasa-blue-marble-200407         NASA Blue Marble NG, July 2004
    moon     nasa-svs-moon-kit-2k            NASA/GSFC SVS CGI Moon Kit (LRO LROC color)
    mars     usgs-mars-viking-mdim21-1km     USGS Viking MDIM 2.1 color mosaic (1 km)
    mercury  nasa-messenger-mercury-pia15063 MESSENGER global mosaic (grayscale)
    jupiter  nasa-cassini-jupiter-pia07782   Cassini map of Jupiter, Dec 2000

The longitude layout was checked against known features: Tycho at -11 and
Mare Crisium at +58 on the Moon; Olympus Mons at -134 and Hellas at +68 on
Mars; the Caloris basin near +160 on Mercury. Jupiter's map does not state
its longitude system; its bands do not depend on it, and its spots drift
against any rotation period anyway. Mercury's grayscale mosaic is stretched
(the 1st to 99.5th percentile of the middle latitudes to the full range) so it is not near black.

    python3 scripts/make_planets.py           # all bodies
    python3 scripts/make_planets.py moon mars # some

Needs Pillow with WebP (and NumPy for Mercury). The output is not byte-for-
byte reproducible across library versions, so the "Data" CI job does not
regenerate it; scripts/check-data.sh only checks the size and format.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import data_sources  # noqa: E402

WIDTH, HEIGHT, QUALITY = 2048, 1024, 80
OUT_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "data")
SOURCES = {
    "earth": "nasa-blue-marble-200407",
    "moon": "nasa-svs-moon-kit-2k",
    "mars": "usgs-mars-viking-mdim21-1km",
    "mercury": "nasa-messenger-mercury-pia15063",
    "jupiter": "nasa-cassini-jupiter-pia07782",
}


def stretch(img):
    import numpy as np
    from PIL import Image

    a = np.asarray(img.convert("L"), dtype=np.float64)
    # From the middle latitudes: the polar rows hold black gaps and glare.
    h = a.shape[0]
    lo, hi = np.percentile(a[h // 8 : h - h // 8], [1, 99.5])
    a = np.clip((a - lo) / max(hi - lo, 1.0), 0.0, 1.0) * 255.0
    return Image.fromarray(a.round().astype(np.uint8)).convert("RGB")


def build(body):
    from PIL import Image

    Image.MAX_IMAGE_PIXELS = None  # the Mars mosaic is 21339x10670
    src = data_sources.fetch(SOURCES[body])
    img = Image.open(src)
    img.load()
    img = stretch(img) if body == "mercury" else img.convert("RGB")
    w, h = img.size
    if not 1.95 <= w / h <= 2.05:
        raise SystemExit(f"{body}: {w}x{h} is not an equirectangular map")
    img = img.resize((WIDTH, HEIGHT), Image.LANCZOS)
    out = os.path.join(OUT_DIR, f"planet_{body}.webp")
    img.save(out, "WEBP", quality=QUALITY, method=6)
    print(f"wrote {out}: {os.path.getsize(out)} bytes", file=sys.stderr)


def main(argv):
    bodies = argv or list(SOURCES)
    for b in bodies:
        if b not in SOURCES:
            raise SystemExit(f"unknown body {b!r}; one of {', '.join(SOURCES)}")
        build(b)


if __name__ == "__main__":
    main(sys.argv[1:])
