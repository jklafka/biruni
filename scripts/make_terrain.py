#!/usr/bin/env python3
"""Build assets/data/terrain.webp, the Terrain layer's texture.

Sources, both Natural Earth 1:50m rasters (public domain), 10800x5400 TIFFs:

  HYP_50M_SR_W  Cross blended hypsometric tints with shaded relief and water
  OB_50M        Ocean bottom: shaded bathymetry, land filled with one flat color

  https://www.naturalearthdata.com/downloads/50m-raster-data/

The URLs, sizes and sha256 are pinned in data-sources.lock.json; the script
fetches them through scripts/data_sources.py (cached in .data-cache/, about
150 MB, verified, an error on any mismatch).

Composite: the ocean bottom is divided by its flat land color and multiplied
into the hypsometric image. On land the factor is 1, so the land is unchanged;
at sea the relief of the sea floor shades the image's depth tint. Both rasters
share Natural Earth's coastline, so no land mask is needed. The result is
resampled to 4096x2048 (Lanczos) and written as a lossy WebP.

Needs Pillow built with WebP support, and NumPy (`pip install Pillow numpy`).

    python3 scripts/make_terrain.py                  # fetch the pinned files, then convert
    python3 scripts/make_terrain.py HYP.zip OB.zip   # convert local copies

The output is not byte-for-byte reproducible across Pillow/libwebp versions,
so the "Data" CI job does not regenerate it; scripts/check-data.sh only
checks that it is a 4096x2048 WebP under 2 MB.
"""
import io
import os
import sys
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import data_sources  # noqa: E402

HYPSO = ("ne-50m-hypso-relief-water", "HYP_50M_SR_W.tif")  # lockfile entry, member
OCEAN = ("ne-50m-ocean-bottom", "OB_50M.tif")
SOURCE_SIZE = (10800, 5400)
# The color OB_50M fills land with. Checked against the Sahara, so a changed
# source fails loudly instead of tinting every continent.
OB_FLAT = (129, 183, 219)
WIDTH, HEIGHT, QUALITY = 4096, 2048, 85
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "data", "terrain.webp")


def read_tif(zip_path, member):
    from PIL import Image

    with zipfile.ZipFile(zip_path) as z:
        names = [n for n in z.namelist() if os.path.basename(n) == member]
        if len(names) != 1:
            sys.exit(f"{zip_path}: expected one {member}, found {names}")
        data = z.read(names[0])
    im = Image.open(io.BytesIO(data)).convert("RGB")
    if im.size != SOURCE_SIZE:
        sys.exit(f"{member}: unexpected size {im.size}, wanted {SOURCE_SIZE}")
    return im


def composite(hypso, ocean):
    """Multiply the ocean bottom's relief (relative to its flat land color) into the hypso image."""
    import numpy as np

    h = np.asarray(hypso, dtype=np.float32)
    o = np.asarray(ocean, dtype=np.float32)
    w, ht = SOURCE_SIZE
    sahara = o[int((90 - 23) / 180 * ht), int((10 + 180) / 360 * w)]
    if tuple(int(v) for v in sahara) != OB_FLAT:
        sys.exit(f"OB_50M land color is {sahara}, expected {OB_FLAT}")
    factor = np.clip(o / np.array(OB_FLAT, dtype=np.float32), 0.0, 2.0)
    return np.clip(h * factor + 0.5, 0, 255).astype(np.uint8)


def full_image(hyp_zip=None, ob_zip=None):
    """The composite at the sources' full 10800x5400 (also used by
    scripts/make_tiles.py). Fetches the pinned sources unless given paths."""
    from PIL import Image

    Image.MAX_IMAGE_PIXELS = None  # 58 MP sources, from a pinned hash
    if hyp_zip is None or ob_zip is None:
        try:
            hyp_zip = data_sources.fetch(HYPSO[0])
            ob_zip = data_sources.fetch(OCEAN[0])
        except (data_sources.DataSourceError, OSError) as e:
            sys.exit(f"error: {e}")
    return Image.fromarray(composite(read_tif(hyp_zip, HYPSO[1]), read_tif(ob_zip, OCEAN[1])))


def main():
    from PIL import Image

    if len(sys.argv) == 3:
        out = full_image(sys.argv[1], sys.argv[2])
    elif len(sys.argv) == 1:
        out = full_image()
    else:
        sys.exit(__doc__)
    out = out.resize((WIDTH, HEIGHT), Image.LANCZOS)
    out.save(OUT, "WEBP", quality=QUALITY, method=6)
    print(f"wrote {os.path.normpath(OUT)}: {os.path.getsize(OUT)} bytes")


if __name__ == "__main__":
    main()
