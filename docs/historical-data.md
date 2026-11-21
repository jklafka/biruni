# Historical data: time-indexed vectors

Design of the data behind the historical layers (bead maps-3sn.2): political borders from 3400 BCE to 2024 (Cliopatria), and coastlines over geological time (the Merdith et al. 2021 plate model). Licenses are in `DATA_LICENSES.md`.

## The problem

A historical layer must draw any moment: a year from 3400 BCE to 2024, or an age back to 1000 million years. Per-year snapshots would repeat the same polygons thousands of times. Cliopatria already describes its data the other way: each record is a polity's geometry for an interval, "FromYear to ToYear, inclusive", and changes only when the borders do. At most 195 polities hold at once (in 2014).

## Format: HIST

One file per layer, in the "HIST" format. `scripts/hist_format.py` documents it, with the encoder, the Python decoder and its tests. `src/history.rs` decodes it in the app, and `scripts/validate_data.py` checks it.

- **Names**, stored once.
- **Rings**, stored once and shared by every geometry that uses them. When a polity's borders change in one place its other rings (islands, exclaves) are reused: in Cliopatria the 3.8 M points of all records come from 1.8 M points of distinct rings. Coordinates are zigzag varint deltas, as in the MAPD format, quantized to `10^-D` degrees with `D` in the header. Three digits (about 110 m) is plenty after simplifying to a few km.
- **Geometries**: lists of ring indices, filled with the even-odd rule, so holes and multipolygons need no extra structure. They are shared too: 2,387 of 13,412 Cliopatria records repeat an earlier geometry.
- **Records**: a name, an inclusive interval `from..=to`, a geometry, a label point (where the name goes, computed once by the converter), and a tag (layer-specific: the fill color and label tier for borders, the plate id for geology).

Drawing time `t` means taking the records with `from <= t <= to` (`Timeline::at`). With about 14,000 records a linear scan takes microseconds. The year control changes `t` and only the visible set changes.

### Two time scales

The header names the scale. Both keep the rule "later is larger", so one interval test serves both:

- **Calendar years** (`Scale::Years`): negative for BCE, with no year 0, Cliopatria's convention (-1 is 1 BCE, followed by 1 CE). `history::add_years` steps across the gap and `history::format_year` prints "44 BCE".
- **Geological time** (`Scale::TenthsMa`): `-(age in tenths of a million years)`, so 251.9 Ma is stored as -2519. Plate-model features have appearance and disappearance ages, which become `from` and `to`.

## Own format, not PMTiles

The same decision as for the vector layers (`docs/tile-streaming.md`, "Vector data: PMTiles or not"), for stronger reasons here:

- A historical view shows the whole world at one moment. Tiles split space, but the selection here is in time. A tiled archive would need the time filter inside every tile, and the same polygon cut across many tiles and zoom levels.
- The data is small once simplified: a few MB for all of Cliopatria, fetched once.
- The decoder is about 150 lines with tests, and it shares MAPD's varint and delta code. PMTiles would need a gzip inflater (about 8 KB of gzipped wasm) plus an MVT decoder.

## Drawing (bead maps-3sn.4)

The political layer already colors countries from an 8-bit ID raster (`u_ids`), a per-ID color table (`u_idColor`) and a palette, and draws borders as vector lines. A historical year reuses all three:

- the records holding then get IDs 1 to 255 (195 at most are needed), rasterized into the ID raster by the worker;
- colors come from a stable hash of the name, so a polity keeps its color from year to year;
- the rings feed the line pass as borders, and names go at the records' label points.

## Geology (bead maps-3sn.7)

The plate model stores present-day coastline polygons, each with a plate id and an age interval, plus a rotation file: finite rotations of each plate relative to another, at given ages. Reconstructing an age means composing each plate's rotations up to the absolute frame, interpolating between the listed ages, and rotating its polygons. That is cheap enough to do in the browser for any age, so the HIST file holds the present-day polygons, with the plate id as each record's tag, and a small rotation table ships alongside.
