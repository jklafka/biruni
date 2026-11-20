# Tile streaming for image layers

Status: implemented for Satellite, Terrain and Night lights (`src/tiles.rs`, `src/tile_stream.rs`, `src/tile_pack.rs`).

## Problem

Each image layer (Satellite, Terrain, Night lights) is one world picture,
4096×2048 on capable desktops and 2048×1024 on the small tier
(`render::choose_budget`). That is about 9.8 km per texel at the equator. At
the closest zoom (about 400 km altitude) a device pixel covers about 0.2 km,
so the picture is magnified roughly 50 times and looks soft.

Sharper whole-world textures don't fit: 16384×8192 RGBA8 is 512 MB before
mipmaps. So the detail has to arrive as tiles, picked for the current view
and loaded when needed, inside a fixed GPU budget.

## What stays the same

- The whole-world picture remains the **base**. It is always resident while
  its layer is shown, keeps its mipmaps and anisotropic filtering, and is the
  fallback wherever no tile is resident. Nothing gets worse without tiles:
  no network, a missing pack, a lost context or a full cache all show today's
  picture.
- Vector lines, labels, the land mask and the country IDs are unchanged.
- The coordinate conventions are unchanged: tiles are equirectangular like
  every other texture (`u = lon/360 + 0.5`, `v = 0.5 − lat/180`, row 0 north).

## Pyramid

Tiles are 256×256. Level `z` covers the world with `2^(z+1)` columns and
`2^z` rows, so it is `512·2^z` texels wide:

| Level | World size | Tiles | Texel at equator |
| --- | --- | --- | --- |
| 2 | 2048×1024 | 8×4 | 19.6 km (small-tier base) |
| 3 | 4096×2048 | 16×8 | 9.8 km (desktop base) |
| 4 | 8192×4096 | 32×16 | 4.9 km |
| 5 | 16384×8192 | 64×32 | 2.4 km |
| 6 | 32768×16384 | 128×64 | 1.2 km |
| 7 | 65536×32768 | 256×128 | 0.6 km |

The base of a tier is level `B = log2(base width / 512)`: 3 on desktops and
2 on the small tier. A tileset stores levels 3 to `M`. Level 3 duplicates the
desktop base but gives the small tier one more step. `M` is the deepest
level that does not upsample the source by more than 1.25×:

| Layer | Source | M | Pack |
| --- | --- | --- | --- |
| Satellite | Blue Marble NG 2004-07 500 m, 86400×43200 in 8 source tiles (`nasa-blue-marble-200407-500m-a1`..`d2`) | 7 | about 117 MB, 43648 tiles |
| Night lights | Black Marble 2016 500 m, 86400×43200 in 8 source tiles (`nasa-black-marble-2016-500m-a1`..`d2`) | 7 | about 44 MB, 43648 tiles |
| Terrain | Natural Earth 1:50m hypso + ocean bottom, 10800×5400 | 4 | 3.2 MB, 640 tiles |

That is 16× sharper than the base for Satellite and Night lights and 2× for
Terrain. Satellite's levels 3..7 take 0.6, 2.2, 7.3, 24.4 and 76.8 MiB, but
a view reads only its own tiles.

NASA publishes the 500 m mosaics as eight 21600×21600 JPEGs, A1 to D2:
letters run west to east from −180°, 1 is north of the equator and 2 south.
Each covers 90°×90°, and from level 1 on, tile edges fall on source tile
edges, so `make_tiles.py` builds one source tile at a time: it pads the tile
with 128 px from its neighbors (wrapping east-west, repeating the edge row
past the poles) and cuts every level's tiles from that with a Lanczos
resample of just the tile's box. This gives the same tiles as resampling the
whole world at once (`test_tile_pack.py` checks it), while memory holds one
source tile instead of a 6.4 GB level 7. Before cutting, each source tile is
compared with its place in the pinned world image the base picture comes
from, so a misnamed or misplaced tile fails the build.

## Tile packs

The tiles of a layer ship as one **tile pack** file, read with HTTP range
requests. Thousands of separate objects would bloat the manifest and the
deploy, and committing them would bloat git.

Layout, little-endian:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | magic `BTIL` |
| 4 | 1 | version, 1 |
| 5 | 1 | min level (3) |
| 6 | 1 | max level `M` |
| 7 | 1 | tile format: 1 = WebP |
| 8 | 2 | tile size (256) |
| 10 | 2 | border (1): texels copied from each neighbor on every side |
| 12 | 4 | tile count `N` = Σ `2^(2z+1)` over the levels |
| 16 | 8·N | directory: `u32` offset, `u32` length for each tile |
| … | | tile bytes |

A stored tile image is 258×258: its 256 texels plus a 1-texel border taken
from its neighbors (wrapping in longitude, repeating the edge row at the
poles). Without it, a tile clamps at its edge, and at the finest level, where
tiles are magnified 10× or more at close zoom, every tile edge shows as a
line. With it, bilinear filtering blends across edges as in one big texture.

The directory and the tile bytes are both ordered by level, then row, then
column. So a row of tiles is one contiguous byte range, and requests for
neighbors can be coalesced. Length 0 means the tile is absent; the renderer
falls back to a coarser level. Sparse packs (test fixtures) use this.

The directory of levels 3..7 is 349 KB, too much to read before the first
tile. So the app reads it in pages of 4096 entries (32 KB): the header and
the first page in one range request, which covers levels 3..5 and part of 6,
then each later page when a wanted tile needs it (at most 2 at once). Because
entries are ordered by level and row, a level 7 page is a band 16 rows (22.5°)
tall, and a close view needs one or two. A failed page makes its tiles absent.
Each tile then comes with its own range request. A server that ignores `Range` and answers 200 still works, because
the reader slices the full body. Cloudflare caches the whole object and
serves ranges from the cache.

### Where packs live

- Packs are built from pinned sources by `scripts/make_tiles.py <tileset>`,
  into the git-ignored `.data-cache/tiles/` under a content-hashed name.
- The `publish-tiles.yml` workflow (manual dispatch) builds the packs on a
  runner, uploads any that R2 lacks to `files/<hashed name>`, checks that
  they are served with CORS and range support, and attaches the updated
  `assets/data/tilesets.json` as an artifact. A follow-up PR commits it.
  WebP bytes are not reproducible across libwebp versions, so a pack's
  identity is the hash recorded at build time, never a rebuild.
- `assets/data/tilesets.json` is an ordinary hashed data file:

  ```json
  {"version": 1, "tilesets": [{"layer": "imagery",
    "url": "https://biruni-data.jklafka.cloud/files/imagery.0123456789abcdef.tiles",
    "sha256": "…", "size": 9621036, "min_level": 3, "max_level": 5,
    "tile_size": 256, "format": "webp"}]}
  ```

  The URLs are absolute, so local builds and preview deploys stream the
  production packs. `validate_data.py` checks the file's schema. An empty
  list is valid: the app then shows only the bases.
- Packs are not in `dist/`, so the 25 MiB Pages limit does not apply. R2
  objects are never deleted, so a pack under a hashed name never changes.

## Tile selection (CPU, browser-free)

`src/tiles.rs`, unit tested. On each frame in which the camera changed, sample
the screen on a grid (every 48 CSS pixels, plus the edges). At each sample,
`Camera::pick` the point and two neighbors one step right and down. From the
three lat/lon pairs, compute the uv derivatives per device pixel with the same
polar factor as the shader (`u` scaled by `max(cos lat, 0.02)`, longitude
differences wrapped). Then, at the finest level `M` (world width `W`):

```
rho = max(length(duv_dx * (W, W/2)), length(duv_dy * (W, W/2)))   texels per device pixel
d   = M - log2(rho) - bias                                          continuous level
```

The sample wants the tile at `zf = ceil(d)` and its parent at `zf − 1`, both
clamped to `[B+1, M]`; nothing if `d ≤ B`. The union over all samples is the
wanted set, ordered coarse levels first, then by distance from the screen
center, and cut to 80% of the cache. A crowded view (a whole globe on a large
high-DPI screen, or the poles, where equirectangular tiles are narrow on the
ground) so keeps the coarse and central tiles, and the shader shows the rest
from resident parents or the base. `bias` is a fixed constant
(`tiles::LOD_BIAS`, 0), shared with the shader.

The shader computes `d` with the same formula. It never needs the CPU's
choice: it looks up whatever is resident.

## GPU cache

- **Tile array:** a `TEXTURE_2D_ARRAY`, 258×258 RGBA8, one tile per layer,
  `LINEAR` filtering without mipmaps, `CLAMP_TO_EDGE`. Created when an image
  layer first needs a tile. Capacity: 96 tiles (24.4 MB) on desktops and
  32 (8.1 MB) on the small tier, from `choose_budget`.
  - Why no mipmaps: tiles are only sampled at most 2× minified (see below),
    the same range trilinear filtering samples a mip level in.
  - Why an array, not an atlas: each layer clamps at its own edges, so
    neighbors in the cache never bleed into each other; the stored border
    supplies the texels across each edge.
- **Page table:** an `RGBA8UI` texture with one mip level per pyramid level
  `B+1..M`. Mip `k` is pyramid level `M − k`, `2^(M+1−k)×2^(M−k)` cells (64×32
  at level 5, 11 KB for the whole chain). The cell for tile `(z, x, y)` holds
  the cache slot (R + 256·G) and level (B) of the **finest resident tile at
  level ≤ z** that covers it, or level 0 for none. The CPU rebuilds the whole
  chain whenever residency changes and uploads it. It is a few thousand cells,
  which is cheap. Rebuild order goes coarse to fine: each cell inherits its
  parent's entry unless its own tile is resident.
- **LRU:** the cache keeps `(tileset, z, x, y) → slot` with a last-wanted frame
  number. A new tile takes a free slot, else the least recently wanted slot
  that is not in the current wanted set. If every slot is wanted, the tile
  waits; the bias step above makes this rare.

## Shader

In `globe.frag`, after the base sample, and outside any derivative-dependent
control flow:

```
d  = M - log2(rho) - bias                (rho from duvdx, duvdy at width W)
zf = ceil(d), zc = zf - 1, wc = zf - d   (trilinear weights)
detail(z): if z <= B, the base sample. Otherwise texelFetch the page table at
           (cell of u,v at level z, mip M - z) and get the slot and level zr.
           If zr == 0, the base sample. Otherwise textureLod(tiles,
           vec3(fract(uv * cells(zr)), slot), 0).
imagery = mix(detail(zf), detail(zc), wc)
```

- `detail` uses only `texelFetch` and `textureLod`, so it may sit inside
  `if (u_layer == 2 && u_tilesOn > 0.5 && d > B)` without breaking the 2×2
  quad rule.
- The seam: the cell index uses `fract(u)`, so either longitude
  parameterization (`u1`/`u2`) and flat maps (where `u` runs past 0..1) land on
  the same tile, and explicit LOD needs no derivative across the seam.
  `rho` uses the seam-safe `duvdx`/`duvdy` that already exist.
- The poles: `v` is clamped to `[0, 1 − ε]` for the row index. `rho` includes
  the polar `u` factor, so the level follows the real footprint. Because a
  tile clamps at its edge, the pole row has no wrap artifacts.
- Morph (mode 2) uses the same uv and derivatives, so it works unchanged.
- Uniforms: `u_tiles` (sampler2DArray), `u_pages` (usampler2D), `u_tilesOn`,
  `u_tileLevels` (vec3: B, M, bias), plus `u_tileWidth` (`512·2^M`). They go
  in the `render.rs` uniform list test.

## Loading

`src/tile_stream.rs`, with a browser-free core and the `fetch` and decode glue
around it:

- When an image layer is shown and it has a tileset, fetch the header and the
  first directory page once (`Range: bytes=0-(16+8·min(N, 4096)−1)`), and check
  the magic, version, levels and count against `tilesets.json`. On failure, log
  it once and keep the base. Later directory pages are fetched when a wanted
  tile falls in them.
- Each frame in which the camera changed: select, then start fetches for wanted
  tiles that are neither resident nor in flight. At most 6 run at once, coarse
  and central first. Queued tiles that are no longer wanted are dropped before
  they start. A fetch in flight finishes, and its tile is cached if a slot is
  free.
- A tile's bytes go through `createImageBitmap` (decoded off the main
  thread), then `texSubImage3D` into its slot, then a page table rebuild, then
  `dirty`. One 256×256 upload is about 0.25 MB, well under the 50 ms task
  budget. Encoded bytes are not kept.
- Switching layers: the page table is cleared at once (the new layer's tiles
  aren't resident). Slots of the old tileset are reused by LRU, so switching
  back soon may still find tiles.
- Context loss: drop the array and the page table, and mark nothing resident.
  On restore, recreate them empty. The next selection fetches again, mostly
  from the HTTP cache.
- Redraw: only tile arrivals and camera changes set `dirty`. An idle view does
  no work once its tiles are in.

## Testing

- `tiles.rs`: level formula against hand values; the seam (views across
  ±180° want tiles from both edges); the poles; flat projections; NaN and
  infinite picks are ignored; budget bias; ordering; page table inheritance.
- Pack parsing in Rust and Python against the same fixture bytes; the
  Python writer round-trips; truncated or wrong-level packs are rejected.
- `validate_data.py`: `tilesets.json` schema rules.
- Visual: `run.mjs` serves a committed sparse fixture pack (real Satellite
  tiles around the Alps and across the antimeridian) in place of the
  production URL and aborts other pack URLs. That keeps the screenshots
  independent of the network. New views: Satellite at close zoom over the
  Alps (sharp), across the seam, and context restore with tiles.

## Follow-ups

- Terrain stops at level 4 (Natural Earth 1:50m). Natural Earth's 1:10m
  rasters (21600 wide) would give level 5.
- Coalesce range requests for neighboring tiles in a row.
- The same machinery for planet surfaces (`maps-ztx.6`). HiPS sky tiles
  (`maps-4nv.7`) need a HEALPix page table, not this equirectangular one.
- PMTiles for large vector data (`maps-wc7.3`) is separate: vector tiles need
  a different cache.
