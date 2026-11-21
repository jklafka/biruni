# AGENTS.md

These are the guidelines for AI coding agents (and humans) working in this repository.

## Project

`biruni` (Biruni) is a web app for exploring maps. It is written in Rust, compiled to WebAssembly, and rendered with WebGL2. There is no JavaScript framework and no backend. The build output is a static site.

| Path | Purpose |
| --- | --- |
| `src/main.rs` | Entry point: DOM wiring, translating DOM input into `controls::Event`s, the animation loop, URL-hash view state |
| `src/controls.rs` | Browser-free input state machine: drag, pinch, fling, wheel/button zoom, fly-to, reset. Mutates a `Camera` |
| `src/camera.rs` | Orbit camera math: picking, projection, trackball panning, bearing/north-up lock, zoom anchoring |
| `src/projection.rs` | Flat map projections (forward/inverse math), the `Projection` enum and the globe <-> flat morph timeline |
| `src/geodesic.rs` | WGS84 inverse geodesic distance (Vincenty, with Karney-style bisection near the antipode). Measure lengths use it; the drawn path and the scale bar stay spherical |
| `src/sky.rs`, `src/sky_render.rs`, `src/shaders/sky.frag` | The sky view: browser-free sky camera (azimuth, altitude, field of view), equatorial and horizontal coordinates, the stereographic projection, gestures and the `&view=sky&look=` hash; its WebGL passes; and the background shader (sky, ground, horizon, RA/Dec grid) |
| `src/constellations.rs` | Constellation figures of the sky view: decoding `constellations.bin`, great-circle screen polylines split at the horizon and trimmed short of their stars, and names placed clear of the star names |
| `src/stars.rs`, `src/shaders/stars.vert`, `src/shaders/stars.frag` | Stars of the sky view: decoding `stars.bin`, B-V colors, the magnitude limit for a field of view, bright star name labels; drawn as additive points with the sky projection |
| `src/ephemeris.rs` | Browser-free planet positions from JPL's approximate Keplerian elements (Standish and Williams), 3000 BC to 3000 AD; tests compare against JPL Horizons. The sky view places the Sun and planets with it |
| `src/healpix.rs`, `src/survey.rs`, `src/shaders/survey.vert`, `src/shaders/survey.frag` | The sky view's deep-sky survey (DSS color HiPS, off by default): HEALPix nested geometry (faces, tiles, cones; the tile image orientation was measured against bright stars), and the browser-free planner (mirror URLs, which order to stream, the Allsky atlas and tile meshes, LRU tile cache); drawn with the stencil so each part of the sky comes once from the finest layer there |
| `src/moon.rs`, `src/sky_bodies.rs` | The Moon (Meeus ch. 47 / ELP-2000/82 main terms, precessed to J2000, topocentric parallax; tests against Meeus's example and JPL Horizons, 0.013° worst from 1900 to 2100), and the sky view's Sun, Moon and planets: directions, sizes, the Moon's phase, screen placement and labels, and the daylight factor from the Sun's altitude |
| `src/view.rs` | HUD text formatting and the `#lat,lon,alt_km[,bearing]&layer=&proj=` URL hash |
| `src/geo.rs` | Decodes the compact binary map layers (`decode_shapes`); rasterizes the land mask into the map texture with Canvas 2D, in the worker when possible |
| `assets/raster-worker.js` | Web Worker (OffscreenCanvas) that rasterizes the land mask off the main thread. Mirrors `geo::draw_tile`, `geo::draw_ice` and the binary decoder. Copied to `dist/` by Trunk |
| `src/lines.rs` | Pure geometry for vector coastlines and borders: seam splitting, Douglas–Peucker levels of detail, segment building |
| `src/labels.rs` | Browser-free label selection: altitude gate, horizon test, priority, greedy collision, fade steps |
| `src/label_layer.rs` | Draws labels on the `#labels` canvas (no pointer events), fade state, loads `labels.tsv` |
| `src/credits.rs`, `src/credits_ui.rs` | Data credits panel: browser-free table of datasets and `visible_credits(layer)` (a test keeps it in step with the "In use" table of `DATA_LICENSES.md`), and the DOM glue for the `#credits` dialog. Add new datasets that need credit to both |
| `src/scrub.rs`, `src/scrub_ui.rs` | Day/night time scrubber: browser-free UTC time math and play stepping, and the panel's DOM glue. Panels that sit bottom left offset themselves by `--scrub` in `index.html` |
| `src/tiles.rs`, `src/tile_stream.rs`, `src/tile_pack.rs` | Detail tiles for the image layers (`docs/tile-streaming.md`): browser-free pyramid math, tile selection, page table and slot cache; the streaming planner plus range fetch and decode glue; the tile pack directory parser |
| `src/render.rs` | WebGL2 setup, shader compilation, texture upload, line pass and globe draw call |
| `src/shaders/` | GLSL ES 3.00. `globe.frag` ray-traces the globe for each pixel, or inverts a flat projection (`u_mode`). `morph.vert` draws the globe <-> flat morph mesh. `line.vert`/`line.frag` draw coastlines and borders (in every projection) as screen-space-width lines into an offscreen target that `globe.frag` reads. |
| `assets/data/` | Compact `.bin` map layers generated from `data-src/`, copied to `dist/data/` by Trunk, then moved to content-hashed names under `dist/data/files/` with a `manifest.json` by `scripts/hash_data.py` (a Trunk post-build hook) |
| `scripts/publish_data.py`, `scripts/r2-cors.json` | Helpers for `deploy.yml`'s publish-to-R2 steps on main (fetch the live data as a validation baseline, list files R2 lacks, rewrite the manifest to absolute R2 URLs) and the bucket's CORS rules. Tested by `scripts/test_publish_data.py` |
| `src/data.rs` | Data files by name: loads `data/manifest.json` once and resolves each name to its hashed URL (`data::fetch`) |
| `docs/tile-streaming.md` | Design of tile streaming for the image layers: pyramid, tile pack format, selection, GPU cache and page table, shader lookup |
| `.github/workflows/publish-tiles.yml` | Manual workflow: builds tile packs from pinned sources, uploads them to R2, attaches the updated `assets/data/tilesets.json` |
| `DATA_LICENSES.md` | License, required credit and status of every dataset in use or planned; the policy for share-alike and non-commercial data |
| `data-src/` | Natural Earth GeoJSON sources (public domain). Not served; tests use them as the reference |
| `build.rs`, `shader_minify.rs` | Strip comments and indentation from `src/shaders/*` into `OUT_DIR` before `include_str!` embeds them (the minify function is unit tested in `render.rs`) |
| `data-sources.lock.json` | Lockfile of pinned upstream data: name, URL, version, sha256, size and license key for each source whose bytes are not committed |
| `scripts/` | `data_sources.py` fetches, verifies and bumps the lockfile entries (`test_data_sources.py` tests it); `convert_geojson.py` converts `data-src/*.geojson` into `assets/data/*.bin`, `places.tsv` and `labels.tsv`; `marine_label_points.py` derives ocean label points from the Natural Earth marine shapefile; `make_blue_marble.py` builds the satellite imagery `assets/data/blue_marble.webp` from the pinned NASA JPEG; `make_terrain.py` builds the Terrain layer `assets/data/terrain.webp` from two pinned Natural Earth rasters; `make_night_lights.py` builds the Night lights layer `assets/data/night_lights.webp` from the pinned NASA Black Marble JPEG; `make_tiles.py` builds the image layers' tile packs (format in `tile_pack.py`, tested by `test_tile_pack.py` against `scripts/testdata/tiny.tiles`) and updates `assets/data/tilesets.json` |
| `scripts/validate_data.py` | Rule-based validation of a built data directory (`dist/data`): manifest and hashes, size limits, a Python decoder for every file type, optional shrink guard against a baseline. CI and the deploy workflow run it after the release build |
| `.github/workflows/deploy.yml` | Builds the release site and deploys it to Cloudflare Pages (production on `main`, previews on PRs) |
| `assets/_headers` | Cloudflare Pages headers (wasm MIME type, caching), copied to `dist/` by Trunk |
| `visual-tests/` | Playwright + pixelmatch screenshot tests; `baselines/` holds the committed reference PNGs |
| `.github/workflows/ci.yml` | CI: format, clippy, tests, shader validation, release build, visual regression |
| `index.html` | Page shell, HUD and controls. Trunk reads it as the build entry. |

## Setup and commands

You need rustup with the `wasm32-unknown-unknown` target, [Trunk](https://trunkrs.dev), and Python 3 (Trunk runs `scripts/hash_data.py` after every build).

```sh
trunk serve                 # dev server with live reload at http://127.0.0.1:8080
trunk build --release       # production build in dist/
cargo test                  # unit tests (run natively, not in wasm)
cargo clippy --target wasm32-unknown-unknown -- -D warnings
cargo fmt
```

If Homebrew's `rust` and `rustup` are both installed, Homebrew's `cargo` may come first on `PATH`, and it has no wasm target. In that case, prefix commands with `PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"`.

## Git workflow

- **Never push directly to `main`.** Every change goes through a pull request.
- Create a short, descriptive branch with a prefix, such as `feat/flat-map-mode`, `fix/pinch-jump` or `docs/readme`.
- Keep each PR focused on one feature or fix. Split unrelated changes into separate PRs.
- Write commit messages in the imperative mood ("Add place search"), with a summary line of about 72 characters or less. Add a body when the reason for the change isn't obvious.
- The PR description says what changed, why, and how it was verified. Include a screenshot for visual changes. If it resolves a GitHub issue, include `Fixes #N` (see **GitHub issues and beads**).
- Do not add attribution lines to commits or PR descriptions: no `Co-Authored-By` trailers and no "Generated with" footers.
- Before opening a PR, make sure every check in **Before you open a PR** below passes.
- Never commit `dist/` or `target/`. Never commit secrets or API keys.

## Before you open a PR

CI (`.github/workflows/ci.yml`) runs the checks below on every pull request. Branch protection on `main` requires all of them to pass, plus a PR, before merging. Run them locally first:

1. **Format:** `cargo fmt --all --check`
2. **Clippy:** `cargo clippy --target wasm32-unknown-unknown -- -D warnings` and `cargo clippy --all-targets -- -D warnings`
3. **Test:** `cargo test --all-targets`
4. **Shaders:** `glslangValidator src/shaders/*.vert src/shaders/*.frag` (from `brew install glslang` or `apt install glslang-tools`)
5. **Data:** `scripts/check-data.sh` (generated files in `assets/data/` match `data-src/`; also runs the Python script tests)
6. **Build:** `trunk build --release`
7. For rendering or interaction changes, look at the result in a browser. The URL hash sets the view, so you can test a specific place reproducibly, e.g. `http://127.0.0.1:8080/#46.8,8.2,600` (lat, lon, altitude in km). Check at least three views: the whole globe, a mid zoom, and the closest zoom (about 400 km). If you can't look at it, say so in the PR. Don't claim it works.
   `cd visual-tests && npm test` (after `trunk build --release`) answers tile pack requests from `visual-tests/fixtures/<layer>.tiles` (sparse packs from `make_tiles.py fixture`) and renders five fixed views headlessly into `visual-tests/actual/`. Read those PNGs to check your change, and see `diff/` for what moved. Baselines must come from Linux CI, not your Mac: for an intentional change, push your branch, wait for CI, then run `scripts/update-baselines.sh <PR number>` (add `--push` to push). It copies `actual/*.png` from the PR's `visual-screenshots` CI artifact into `visual-tests/baselines/` and commits them as you. Review the new PNGs before pushing (see README). Shared build steps live in `.github/actions/`, so `ci.yml` stays consistent.

Don't weaken or skip a check to get a PR through. Fix the cause. If a check really is wrong, change it in its own PR and explain why.

## Testing

- Tests run natively (`cargo test`), not in wasm, so tested code must not call browser APIs. Put pure logic in browser-free functions:
  - `camera.rs`: projection, picking and panning math
  - `controls.rs`: gestures and animation. Put new interaction logic here, not in `main.rs`
  - `view.rs`: HUD text and URL-hash parsing
  - `geo::decode_shapes`: binary map layers → shapes (the test-only GeoJSON `parse_shapes` is the reference)
- Every bug fix comes with a test that fails without the fix.
- New math or parsing comes with tests, including edge cases: poles, the ±180° seam, NaN/∞, malformed input.
- To test WebGL context recovery by hand, load the app with `?debug=ctxloss` and press the backquote key to lose the context, again to restore it (see `README.md`, "Debug aids"). The `ctxloss-restored` visual test does this; new GPU state that is rebuilt in `Renderer::context_restored` should show up correct in that view. Don't add user-visible controls for debug aids.
- `render.rs` checks that every uniform Rust looks up is declared in `globe.frag`. When you add a uniform, add it to that list.
- `geo.rs` checks that the bundled datasets decode, stay within lon/lat bounds and match the GeoJSON within the quantization tolerance.

## Code guidelines

### Rust

- Target stable Rust, edition 2021. Keep the dependency list small. Before adding a crate, make sure it compiles to `wasm32-unknown-unknown` and is worth the added wasm size. The release wasm is currently about 318 KB (about 147 KB gzipped with `gzip -9`), measured from `trunk build --release`. It has grown with each feature (place search, labels, measuring, the time scrubber, country info), so re-measure rather than trust this figure, and the build was already size-optimized. Release size comes from `opt-level = "z"`, `lto`, `codegen-units = 1` and Trunk's `wasm-opt -Oz --converge`, which are already the smallest settings (`"s"` is about 8% larger, `"3"` about 28% larger; `panic = "abort"` and `strip` change nothing on this target). `build.rs` (via `shader_minify.rs`, which the `render.rs` tests also check) strips comments and indentation from the GLSL before it is embedded, keeping line numbers so shader errors still point at the right line. Together with `--converge --strip-producers` this saves only about 2.6 KB gzipped. Measure with `twiggy` on a wasm built with names, not on the optimized file.
- Add new `web-sys` APIs by enabling the matching feature in `Cargo.toml`.
- Don't use `unwrap()` on anything that can fail at runtime in the browser (DOM lookups, fetches, WebGL calls). Return `Result<_, JsValue>` and surface errors to the user, as `start()` does with the `#loading` panel.
- Keep pure math, such as camera and projection code, free of `web-sys`, so it can be unit tested natively. New math gets tests.
- Event listeners are added with the `listen` helper and live for the whole page (`Closure::forget`). Don't leak closures in code paths that run repeatedly.

### Coordinate conventions (must stay in sync)

`src/camera.rs` and `src/shaders/globe.frag` implement the same math and must agree exactly:

- World space: +Y is the north pole, longitude 0° lies on +Z, and 90°E lies on +X. The globe has radius 1.
- Camera space: the camera sits at `(0, 0, 1 + alt)` looking down −Z. The orientation is the view center (`lat`, `lon`) plus a `bearing`: the compass direction at the top of the screen, clockwise from north. Bearing 0 is north up; screen-up is `cos b · north + sin b · east`. The shaders get only the camera-to-world matrix `rotation()` (`u_rot`), so they work at any orientation, poles included; don't add lat/lon-based camera math to them.
- The field of view (40°) spans the **shorter** viewport axis.
- The texture is equirectangular: `u = lon/360 + 0.5`, `v = 0.5 − lat/180`, with row 0 at the north.

If you change one side, change the other in the same PR, and extend the round-trip tests in `camera.rs`.

The sky view (`src/sky.rs`, `src/shaders/sky.frag`) has its own conventions, documented at the top of `sky.rs` and tested there:

- Equatorial space (J2000 RA/Dec) mirrors world space: +Y is the north celestial pole, RA 0h is on +Z, RA 6h on +X.
- Horizontal space: +Y is the zenith, north is on −Z and east on +X (right-handed, so `sky::eq_to_horizon` is a rotation). Azimuth runs from north through east.
- The sky camera looks down −Z with no roll (the zenith is up the screen); its view is an azimuth, altitude and field of view across the shorter axis.
- The projection is stereographic about the view center: a direction at angle `a` lands at `tan(a/2) / tan(fov/4)` half-short-axes from the center. `SkyCamera::project` / `pick` and `sky.frag` must agree.
- The observer stands at the Earth camera's center at the time scrubber's time. Catalog positions are J2000 and not precessed (about 0.36° off by 2026).

Flat projections follow the same discipline, in `src/projection.rs` (forward and inverse) and the GLSL in `globe.frag` (inverse), `morph.vert` and `line.vert` (forward):

- Plane units: the sphere has radius 1, `x` is measured from the central meridian (`Camera::lon`) and `y` from the equator, north up. A flat camera has `lat` (the row at the screen center), `lon` (the central meridian) and `alt` (the zoom; a flat map is drawn `FLAT_ZOOM` times the globe's size at the same altitude). Its bearing is always 0.
- The projection constants and tables (Robinson, Mercator's limit, Mollweide's scale) appear in Rust and in the shaders, and tests compare them. `flatForward` and `place` must stay textually identical in `morph.vert` and `line.vert`. To add a projection, add it to `Projection`, `shader_index`, all three shaders (the ids must match) and the picker in `index.html`, and extend the tests (round trip, the seam, poles, NaN and infinity).
- Anything that needs a position on screen goes through `Camera::project` / `Camera::pick`, never through its own projection math.

### Shaders

- Write GLSL ES 3.00 for WebGL2. Don't rely on extensions without a fallback.
- Don't return early in the fragment shader before `fwidth`, `dFdx` or `dFdy`, or before implicit-LOD `texture()` calls. Derivatives need every pixel in the 2×2 quad to follow the same path.
- Keep line widths, such as the graticule, coast and borders, constant on screen. The graticule scales with `fwidth`. Coast and borders are vector segments whose width is set in pixels in `line.vert`; their camera math must match `globe.frag`.
- Watch the longitude seam at ±180°. Mipmapped lookups use the two-parameterization trick (`u1`/`u2`) to avoid a visible seam.

### Rendering and performance

- The projection morph counts as motion for the dynamic resolution (`quality.rs`), and `prefers-reduced-motion` switches projections at once instead of animating.
- Only redraw when something changed. The frame loop sets `dirty`. Don't add work that runs every frame while the app is idle.
- Texture sizes come from `render::choose_budget` (pure, unit tested). Capable desktops: map 8192×4096 R8 (land mask) with mipmaps (about 43 MB) plus a 4096-wide country-ID raster (8 MB) and a 4096×2048 glacier mask (R8, mipmapped, about 11 MB). Touch devices, `deviceMemory` ≤ 4 GB and GPUs limited to 4096: map 4096×2048 (about 11 MB) plus a 2048-wide ID raster (2 MB) and a 2048×1024 glacier mask (about 3 MB), 16 MB total (the image layers, Satellite, Terrain or Night lights, share one 11 MB texture, allocated only when one of them is opened; never keep two image textures at once). Image layers add a detail tile array (`docs/tile-streaming.md`), allocated with the first tile: 96 slots (24.4 MB) on desktops, 32 (8.1 MB) on the small tier. The sky view adds its Milky Way (2048×1024 RGBA8, mipmapped, about 11 MB, on texture unit 7) and its star buffer (41,486 stars × 28 bytes, 1.2 MB) the first time it opens. Its deep-sky survey, only when switched on, adds the Allsky atlas (1728×1856 mipmapped, about 17 MB; half size, 4 MB, on the small tier) and up to 96 (small tier 32) streamed 256×256 tiles at 256 KB each. `?tex=low|high` forces a tier for testing. Keep the small tier under 25 MB, and don't increase either tier without a plan for mobile.
- Don't add rasterizing or other multi-millisecond work to the main thread during load; keep each main-thread task under about 50 ms. The land mask and the glacier mask are drawn in the worker, and `assets/raster-worker.js` must stay pixel-identical with `geo::draw_tile` and `geo::draw_ice` (the main-thread fallback is the reference).
- Rasterize on canvases of 4096×4096 or smaller, because some mobile browsers cap a canvas at 16.7 MP. That's why the map is drawn in two halves.
- Debounce `history.replaceState` (see `hash_stale` and `last_hash`), because Safari throttles it.

### UI

- Every interaction needs a mouse, touch and keyboard path where reasonable. Keep the controls table in `README.md` up to date.
- Keep the UI usable at phone widths. The canvas uses `touch-action: none`, and gestures are handled in Rust.

## Data

- Map data comes from [Natural Earth](https://www.naturalearthdata.com/) (1:50m), which is public domain. If you add another dataset, check that its license allows redistribution, record it in `DATA_LICENSES.md` (which also sets the policy for share-alike and non-commercial licenses), credit it in `README.md`, and add it to `src/credits.rs` so the in-app credits panel lists it.
- Edit the GeoJSON in `data-src/`, never the `.bin` files. After changing it, or the format, run `python3 scripts/convert_geojson.py` and commit the regenerated `assets/data/*.bin`. If you change the format, update `decode_shapes`, the script, the format doc in `geo.rs` and the decoder in `assets/raster-worker.js` together and bump the version byte.
- CI (job "Data") runs `scripts/check-data.sh`, which regenerates everything into a temp dir and compares it byte-for-byte with `assets/data/` (`*.bin`, `places.tsv`, `labels.tsv`). Run it locally before opening a PR that touches `data-src/` or `scripts/`. The converter is stdlib-only Python 3 and must stay deterministic across platforms (no reliance on dict/set order, locale or newline translation; write bytes). `data-src/ne_50m_marine_label_points.geojson` is a committed source: `scripts/marine_label_points.py` needs a separate Natural Earth download, so it is not part of the check.
- Pinned sources: git holds the truth about which upstream data ships, in `data-sources.lock.json` (name, URL, version, sha256, size, and a `license` key that matches a `[key: ...]` tag in a `DATA_LICENSES.md` dataset row). The bytes of large sources are not committed. `python3 scripts/data_sources.py fetch <name>` downloads into the git-ignored `.data-cache/`, checks size and sha256, and fails on any mismatch; builds must get such sources only this way. A refresh is a PR that runs `python3 scripts/data_sources.py bump <name> [--url U] [--version V]` and commits the changed lockfile entry. To add a source, add its license key to `DATA_LICENSES.md`, add an entry to the lockfile (keep it sorted by name) and run `bump`.
- Fixtures versus pinned sources: `data-src/` (Natural Earth 1:50m) stays committed and is not in the lockfile; unit tests and visual baselines use it. `scripts/check-data.sh` never uses the network. It regenerates outputs from committed sources byte for byte, and for the lockfile only runs `data_sources.py check` (schema, sorted unique names, sha256 and size shape, https URLs, license keys) and the script tests.
- Built data is validated by `python3 scripts/validate_data.py dist/data` (after `trunk build --release`; CI "Build" and the deploy workflow run it). It checks the manifest and content hashes, the 25 MiB per-file limit, and decodes every file: `.bin` (MAPD and MAPC, mirroring `decode_shapes` and `decode_countries`), `.tsv` and `.webp`. An unknown file type fails, so a new data type must add a rule there and a test in `scripts/test_validate_data.py`. `--baseline DIR` (the published data) refuses a file that disappears or loses over half its bytes or features unless named with `--allow-shrink`; the publish workflow uses it. If you change a binary format, update the validator's decoder with `decode_shapes`/`decode_countries`.
- `assets/data/blue_marble.webp` (NASA Blue Marble, credited in `README.md`; source pinned as `nasa-blue-marble-200407`) is not generated from `data-src/`: CI does not download it, so `scripts/check-data.sh` only checks its format and size. Rebuild it with `scripts/make_blue_marble.py`. Its decoded GPU memory (44.7 MB high tier, 11.2 MB low tier) is allocated only when the Satellite layer is first selected. `assets/data/terrain.webp` (Natural Earth hypso and ocean bottom rasters, sources pinned as `ne-50m-hypso-relief-water` and `ne-50m-ocean-bottom`) is handled the same way: rebuilt with `scripts/make_terrain.py`, shape-checked only, and drawn into the same texture as the satellite image. `assets/data/stars.bin` (the sky view's stars, HYG v4.4, pinned as `hyg-v44`, CC BY-SA 4.0 so kept in its own file) is built by `scripts/convert_stars.py` (format documented there, tested by `scripts/test_convert_stars.py`, which also writes `scripts/testdata/tiny_stars.bin` for `src/stars.rs`); `check-data.sh` only checks that it decodes and looks sane, and `validate_data.py` decodes it. `assets/data/milky_way.webp` (the sky view's Milky Way, 2048×1024, from NASA SVS Deep Star Maps 2020 pinned as `nasa-svs-milkyway-2020-4k`, an OpenEXR file) is built by `scripts/make_milky_way.py` (needs OpenEXR, NumPy and Pillow) and shape-checked like the image layers. `assets/data/constellations.bin` (Stellarium `western` figures pinned as `stellarium-western`, with HYG positions; CC BY-SA, its own file) is handled the same way by `scripts/convert_constellations.py` and `scripts/test_convert_constellations.py` (fixture `scripts/testdata/tiny_constellations.bin`). So is `assets/data/night_lights.webp` (NASA Black Marble 2016, source pinned as `nasa-black-marble-2016`, rebuilt with `scripts/make_night_lights.py`). Their tile packs (`docs/tile-streaming.md`) are built from the full-resolution sources (Satellite and Night lights from NASA's 500 m mosaics, eight pinned 21600×21600 tiles each, `nasa-blue-marble-200407-500m-a1`..`d2` and `nasa-black-marble-2016-500m-a1`..`d2`) by the Publish tiles workflow and live only on R2; `assets/data/tilesets.json` lists them and is schema-checked by `check-data.sh` and `validate_data.py`. Never commit a pack, and never point `tilesets.json` at a pack that is not on R2 (the deploy checks).
- Keep large data out of the wasm binary. Fetch it at runtime from `assets/` instead of using `include_str!`/`include_bytes!`.
- On pushes to `main`, `deploy.yml` uploads the hashed files to R2 and ships a manifest of absolute URLs (see `README.md`, "Map data on R2"), so the data may come from another origin: fetch it only with `data::fetch` (plain CORS `fetch`), never by a relative path, `scripts/r2-cors.json` allows every origin (`*`), so the site, PR previews and the local dev server all read it. Add a content type to `publish_data.py` when a new file extension appears in `assets/data/`. R2 files are never deleted.
- Fetch data files with `data::fetch("<name in assets/data>")`, never by a `data/...` path: the deployed files have content-hashed names that only the manifest knows. The hashed files are cached as immutable, so never serve changed bytes under an existing hashed name.
- If you change data files or how they are loaded, note the download size in the PR.

## Documentation

- Update `README.md` when you change commands, controls or architecture.
- Update this file when you change a convention it describes.

## GitHub issues and beads

GitHub issues are the public backlog: the issues that people read and comment on, and that PRs close. Beads (`bd`) is the working layer on top: claiming work, dependencies and agent subtasks. Each GitHub issue is mirrored as a bead whose `External:` field links back to the issue (see `bd show <id>`). The repository is configured with `bd config set github.repository jklafka/biruni`. The token is not stored. Pass it from `gh` on each call.

```sh
# Start of a session: bring new and updated GitHub issues into beads (local only)
GITHUB_TOKEN=$(gh auth token) bd github sync --pull-only

# Re-pull specific issues. The incremental pull misses label-only changes and issues closed on GitHub
GITHUB_TOKEN=$(gh auth token) bd github pull <bead-id>...

# Turn a specific bead into a GitHub issue, or push its changes to the linked issue
GITHUB_TOKEN=$(gh auth token) bd github push --dry-run <bead-id>...
GITHUB_TOKEN=$(gh auth token) bd github push <bead-id>...
```

- Pull freely. Pulling changes only the local beads database.
- Pushing writes to GitHub. Push only named beads, and only when asked or when a bead should become a public issue. Agent subtasks stay local. Don't run `bd github sync` without `--pull-only`: it pushes every bead and rewrites every linked issue.
- Priority comes from GitHub labels of the form `priority::<value>` (double colon): `critical` → P0, `high` → P1, `medium` → P2, `low` → P3, `none` → P4. Issues without one are P2. A pull overwrites the local priority, so change priority by editing the label on GitHub, then pull that issue again.
- To finish work on a mirrored issue, write `Fixes #N` in the PR description. Merging closes the GitHub issue. `bd github sync --pull-only` does not notice issues closed on GitHub, so close the bead with a targeted `GITHUB_TOKEN=$(gh auth token) bd github pull <bead-id>`. Pull one bead per call: passing several IDs has silently done nothing. If you `bd close` the bead first, that doesn't close the GitHub issue.

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:1105d646 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->

<!-- BEGIN BEADS CODEX SETUP: generated by bd setup codex -->
## Beads Issue Tracker

Use Beads (`bd`) for durable task tracking in repositories that include it. Use the `beads` skill at `.agents/skills/beads/SKILL.md` (project install) or `~/.agents/skills/beads/SKILL.md` (global install) for Beads workflow guidance, then use the `bd` CLI for issue operations.

### Quick Reference

```bash
bd ready                # Find available work
bd show <id>            # View issue details
bd update <id> --claim  # Claim work
bd close <id>           # Complete work
bd prime                # Refresh Beads context
```

### Rules

- Use `bd` for all task tracking; do not create markdown TODO lists.
- Run `bd prime` when Beads context is missing or stale. Codex 0.129.0+ can load Beads context automatically through native hooks; use `/hooks` to inspect or toggle them.
- Keep persistent project memory in Beads via `bd remember`; do not create ad hoc memory files.

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md for details and anti-patterns.
<!-- END BEADS CODEX SETUP -->
