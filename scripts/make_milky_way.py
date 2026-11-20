#!/usr/bin/env python3
"""Build assets/data/milky_way.webp, the sky view's Milky Way background.

Source: NASA/Goddard Space Flight Center Scientific Visualization Studio,
"Deep Star Maps 2020" (https://svs.gsfc.nasa.gov/4851), the star-free Milky
Way map in celestial coordinates, 4096x2048 OpenEXR (about 36 MB), made from
Gaia DR2 (ESA/Gaia/DPAC). Pinned in data-sources.lock.json as
`nasa-svs-milkyway-2020-4k`; fetched through scripts/data_sources.py.

The map is equirectangular in J2000 right ascension and declination, RA 0h
at the center and increasing to the left (as the sky is seen from inside),
north up: u = 0.5 - RA / 360, v = 0.5 - Dec / 180, which src/shaders/sky.frag
samples. The linear HDR values are tone mapped: the faintest quarter of the
sky goes to black (so the dark sky stays dark), the 99.9th percentile of
brightness to white, then gamma 2.2; resampled to 2048x1024 (Lanczos) and
written as a lossy WebP. The sky shader adds it faintly under the stars.

    python3 scripts/make_milky_way.py             # fetch the pinned file, then convert
    python3 scripts/make_milky_way.py src.exr     # convert a local copy

Needs OpenEXR, NumPy and Pillow with WebP (`pip install OpenEXR numpy
Pillow`). The output is not byte-for-byte reproducible across library
versions, so the "Data" CI job does not regenerate it; scripts/check-data.sh
only checks that it is a 2048x1024 WebP under 2 MB.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import data_sources  # noqa: E402

SOURCE = "nasa-svs-milkyway-2020-4k"
SOURCE_SIZE = (4096, 2048)
WIDTH, HEIGHT, QUALITY = 2048, 1024, 75
FLOOR_PERCENTILE, TOP_PERCENTILE = 25, 99.9
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "data", "milky_way.webp")


def main():
    import numpy as np
    import OpenEXR
    from PIL import Image

    if len(sys.argv) > 1:
        src = sys.argv[1]
    else:
        try:
            src = data_sources.fetch(SOURCE)
        except (data_sources.DataSourceError, OSError) as e:
            sys.exit(f"error: {e}")
    rgb = OpenEXR.File(src).channels()["RGB"].pixels.astype(np.float64)
    if (rgb.shape[1], rgb.shape[0]) != SOURCE_SIZE:
        sys.exit(f"unexpected source size {rgb.shape[1]}x{rgb.shape[0]}")
    lum = rgb.mean(axis=2)
    floor = np.percentile(lum, FLOOR_PERCENTILE)
    top = np.percentile(lum, TOP_PERCENTILE)
    out = np.clip((rgb - floor) / (top - floor), 0.0, 1.0) ** (1.0 / 2.2)
    img = Image.fromarray((out * 255.0 + 0.5).astype(np.uint8))
    img = img.resize((WIDTH, HEIGHT), Image.LANCZOS)
    img.save(OUT, "WEBP", quality=QUALITY, method=6)
    print(f"wrote {os.path.normpath(OUT)}: {os.path.getsize(OUT)} bytes")


if __name__ == "__main__":
    main()
