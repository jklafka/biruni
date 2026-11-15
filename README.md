# maps

A web app for exploring maps, written in Rust and compiled to WebAssembly.

The first map is an interactive 3D globe. Drag to rotate it, and scroll or pinch to zoom from orbit down to about 400 km.

## Running locally

Prerequisites:

- Rust via [rustup](https://rustup.rs) with the wasm target: `rustup target add wasm32-unknown-unknown`
- [Trunk](https://trunkrs.dev): `brew install trunk` or `cargo install trunk`

```sh
trunk serve --open      # dev server with live reload at http://127.0.0.1:8080
trunk build --release   # optimized static site in dist/
cargo test              # unit tests (run natively)
```

> If `rustup` and Homebrew's `rust` are both installed, make sure rustup's toolchain comes first on your `PATH`. Homebrew's `rust` has no `wasm32` target.

Every pull request runs CI (format, clippy, tests, shader validation, a release build and visual regression tests). `main` only accepts pull requests that pass all of it. See [AGENTS.md](AGENTS.md) for contribution guidelines.

## Visual regression tests

`visual-tests/` screenshots the globe in headless Chromium (software WebGL2 through SwiftShader) at fixed views set by the URL hash: the whole globe, mid zoom, closest zoom, the antimeridian, the monochrome layer, and the day/night terminator in both layers (with the sun pinned by `&t=`). It waits for the map data to load, then compares each screenshot with a committed baseline in `visual-tests/baselines/` using pixelmatch. A view fails if more than 0.5% of its pixels differ noticeably. This tolerates small rasterizer differences but catches a flipped land mask, a black globe or a shifted camera.

```sh
trunk build --release
cd visual-tests
npm ci && npx playwright install chromium   # first time only
npm test                                    # compare with baselines
```

New screenshots go to `visual-tests/actual/` and diff images to `visual-tests/diff/` (both git-ignored). CI uploads them as the `visual-screenshots` artifact.

**Context-loss test.** The `ctxloss-restored` view loads mid-zoom with `?debug=ctxloss`, loses the WebGL context, restores it, waits for the re-render and screenshots. It must match the no-loss `mid-zoom` view (the run compares the two directly) and recovery must not start a Worker or request anything again.

**Updating baselines.** Rendering differs slightly between macOS and Linux, so baselines are generated on Linux, the same as CI. After an intentional rendering change, push the branch and open the PR. Wait for CI to finish (the Visual regression job fails, which is expected), then run this from a clean checkout of the PR branch:

```sh
scripts/update-baselines.sh <PR number>          # commit the CI screenshots locally
scripts/update-baselines.sh <PR number> --push   # ... and push
```

It needs `gh`. It finds the latest completed CI run for the PR's head commit, downloads the `visual-screenshots` artifact, copies `actual/*.png` into `visual-tests/baselines/`, lists what changed and commits it as you (nothing if the baselines already match). It refuses to run if the current branch isn't the PR's branch or the tree is dirty, and it says so if CI has no artifact yet. Look at the new PNGs before you push, then wait for CI to pass on the new commit. The commit is made by you, not a bot, so it doesn't trip the repository's extra-approval rule for unattributed changes. `npm run update` rewrites the baselines locally, which is fine for experiments, but don't commit macOS-generated baselines.

## Deployment

The site is hosted on [Cloudflare Pages](https://pages.cloudflare.com). Cloudflare's build image has no Rust toolchain, so `.github/workflows/deploy.yml` builds `dist/` in GitHub Actions and uploads it with `wrangler pages deploy`. Pushes to `main` publish to production. Pull requests from this repository get a preview deployment, and the preview URL is posted as a PR comment. Fork PRs are not deployed. If the secrets below are missing, the deploy step is skipped and the workflow still passes. `assets/_headers` sets the MIME type and cache headers.

One-time setup for the repository owner:

1. In the Cloudflare dashboard, go to Workers & Pages, then create a **Pages** project named `maps` using **Direct Upload** (any placeholder upload). Set its production branch to `main`. A different name needs a matching change to `PAGES_PROJECT` in the workflow.
2. Create an API token (My Profile, API Tokens) with the permission **Account, Cloudflare Pages, Edit**.
3. Copy your Account ID from the dashboard.
4. In GitHub, under Settings, Secrets and variables, Actions, add the repository secrets `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`.
5. Optionally, add the **Deploy to Cloudflare Pages** job as a required check in the `main` branch protection, and put the live URL here.

## Controls

| Action | Mouse / trackpad | Touch | Keyboard |
| --- | --- | --- | --- |
| Pan (the globe follows your cursor, over the poles too) | drag | one-finger drag | arrow keys |
| Turn the view (bearing); unlocks north-up | `Alt`/`Option` + drag around the screen center | two-finger twist (after about 7° of twist, so a plain pinch never turns the view) | `Q` / `E` turn 15° counterclockwise / clockwise |
| Face north and lock north-up | click the compass (top of the button column) | tap the compass | `F` toggles the lock (turning it on faces north); `R` also does |
| Zoom | scroll wheel, trackpad pinch | pinch | `+` / `-` |
| Fly to a point | double-click (shift + double-click zooms out) | double-tap | |
| Reset view | ⟲ button | ⟲ button | `R` |
| Country info | hover highlights a country; click opens the info panel | tap a country | `I` for the country at the view center |
| Close the info panel | × button | × button | `Esc` |
| Switch map layer | layer picker (top right; under the title on phones) | layer picker | `1`, `2`, or `L` to cycle |
| Day/night terminator | day/night button (bottom right) | day/night button | `N` |
| Share a link to the current view | share button (bottom right) | share button (opens the share sheet where supported, else copies) | `S` |
| Switch scale bar between km and mi | click the scale bar | tap the scale bar | `U` |
| Search for a place | search box (top left; under the layer picker on phones), type, then click a result | search box, tap a result | `/` or `Ctrl`/`Cmd`+`K` to focus, `↑` / `↓` to choose, `Enter` to fly there, `Esc` to clear and leave the box |
| Measure distance | ruler button, then click points; the panel lists each leg and the total (km or mi, follows `U`) | ruler button, then tap points | `M` to start, `Enter` or `Space` to drop a point at the crosshair, `Backspace` to undo, `Esc` to clear and exit |

Hovering a country (desktop) highlights it and shows its name in the HUD. Clicking or tapping a country opens a panel with its flag, name, continent, region, population and ISO codes; clicking the ocean, the × button or `Esc` closes it. The panel opens about a third of a second after the click, once it is clear the click is not the start of a double-click (fly-in); measure points are placed immediately. Hover and click are ignored until the country data has loaded, which happens after the first frame. Tiny countries that are barely visible at the current zoom are hard to hit; zoom in.

The HUD shows a scale bar (a round 1/2/5 × 10ⁿ distance) for the ground distance at the screen center. It is hidden when you are zoomed out far enough (above about 1 globe radius of altitude) that the scale varies too much across the view.

Rotation has two modes. **North-up** (the default) keeps north at the top and pans like a flat map; dragging over a pole tilts through it, after which north points down and the compass shows 180°. Near a pole the view swings quickly as you drag, as meridians converge there, and a sideways drag of the pole itself cannot follow the cursor exactly. **Free** is trackball-style: the surface always follows the cursor and the view never rolls on its own. Turning the view (alt-drag, twist, `Q`/`E`, `F`) switches to free; the compass button turns blue while free. Clicking the compass, `F`, or reset (`R`, ⟲) smoothly turns back to north and locks north-up.

The current view is saved in the URL hash as `#lat,lon,altitude_km`, so you can link straight to a place, e.g. `#46.8,8.2,600`. A fourth field adds the bearing, the compass direction at the top of the screen in degrees, e.g. `#46.8,8.2,600,135`; it is written only when the view is not facing north, so older links keep working. A nonzero bearing opens in free mode. You can center on a pole exactly, e.g. `#90,0,6000` (longitude then picks which meridian runs down the screen in north-up). A non-default layer is added as `&layer=<id>`, e.g. `#46.8,8.2,600&layer=mono`. Switching layers never moves the camera.

The day/night terminator is off by default. `&sun=on` turns it on at load, and `&t=<unix seconds>` pins the sun to a fixed time instead of the clock (the visual tests use this so screenshots are deterministic). Both are read at load and again when you edit the hash by hand (a missing `sun` means off). The app writes them back: while the terminator is on, the hash and the share link include `&sun=on`, and `&t=` only if the time is pinned. In live-clock mode `t` is never written, so the hash does not change every minute. Turning the terminator off removes both from the hash, but a pinned time is remembered in the page and returns if you turn it back on. While on, the sun follows the clock and is refreshed once a minute by a timer. That timer is the only work the app does while idle, and it only redraws when the terminator is on.

## Debug aids

These are for testing, not controls, and have no UI. `?debug=ctxloss` (before the `#` hash) makes the backquote key (`` ` ``) lose the WebGL context through the `WEBGL_lose_context` extension; press it again to restore it. This exercises context recovery (everything is rebuilt from data kept in memory, with no refetch) without the browser console. Without the flag the key does nothing. `?tex=low|high` forces a texture tier.

## How it works

- **`src/camera.rs`**: an orbit camera. Its orientation is the view center plus a bearing, so every orientation including the exact poles is representable. It handles picking (screen point → lat/lon), projection (lat/lon → screen point), and trackball panning that makes the surface follow your cursor; in north-up mode the roll is then snapped back to north (or to south-up after tilting over a pole). The shaders only see its `rotation()` matrix, so they work at any bearing. When you zoom, the point under the cursor stays where it is.
- **`src/geo.rs`**: fetches the compact binary layers (converted from [Natural Earth](https://www.naturalearthdata.com/) GeoJSON) and rasterizes the land mask with Canvas 2D into an equirectangular texture. The rasterizing runs off the main thread in `assets/raster-worker.js`, a small hand-written classic Web Worker that decodes the same binary format and draws with `OffscreenCanvas`, then hands back the pixels (the page stays responsive during load). Without `OffscreenCanvas` or workers it draws on the main thread instead, with identical output. The texture is 8192×4096 on capable desktops and 4096×2048 on touch, low-memory (`navigator.deviceMemory` of 4 GB or less) and 4096-limited GPUs. Append `?tex=low` or `?tex=high` to the URL to force a tier. The texture is a single `R8` channel holding land with lakes cut out. Coastlines and borders are not baked in: they stay vector data.
- **`src/country.rs`**: decodes the country polygons and attribute table (also lazy-loaded), hit-tests a lat/lon against them (bounding-box prefilter, then even-odd point-in-polygon over outer rings and holes), and rasterizes a country ID per texel (`index + 1`, 4096×2048, one byte each) with a scanline fill, in slices that yield to the event loop (about 45 ms in total). `src/info_panel.rs` fills the panel in `index.html`.
- **`src/lines.rs`**: browser-free geometry for coastlines (land and lake outlines) and country borders. Shapes become unit-sphere segments, so the ±180° seam and the poles need no special case. Antimeridian and pole edges that Natural Earth adds when cutting polygons are dropped, then each line is simplified with Douglas–Peucker at four tolerances. The renderer picks the coarsest level whose error stays under about half a pixel at the current altitude and screen size (about 77k coast segments at full detail, 4.9k at the coarsest).
- **`src/render.rs`** and **`src/shaders/`**: WebGL2 draws one full-screen triangle. The fragment shader ray-traces the sphere for each pixel, converts the hit point to lat/lon, and samples the texture. Close up, it thresholds the mask so land edges stay crisp. Before that, a line pass draws the segments as instanced quads, 1 to 1.25 CSS px wide at every zoom and hidden past the horizon, into a screen-sized `RG8` target (coast, borders) that the globe shader reads, so lines get the same lighting, night tint and monochrome colours as the fill. It also adds the graticule, lighting and atmosphere. The country-ID raster is a second `R8` texture (4096 wide, 8 MB, or 2048 wide, 2 MB on the small tier); the shader tints the texels whose ID equals `u_highlight`, bilinearly blended so the edge stays smooth. Hover only changes that uniform, so it redraws once per country change, not per mouse move.
- **`src/sun.rs`**: low-precision solar position (declination, plus equation of time via right ascension minus sidereal time) giving the subsolar point and the world-space sun direction for the `u_sun` uniform. Accurate to about 0.01°.
- **`src/view.rs`**: formats the HUD text, picks the scale bar length, parses the URL hash, and defines the `Layer` enum. A layer is a shader palette selected by the `u_layer` uniform; a layer that needs data declares it with `Layer::data_url`, and `layer_loader` fetches it on first selection.
- **`src/layer_loader.rs`**: browser-free state machine for lazy layer data (idle, loading, ready, failed). While a layer loads, the globe keeps the previous layer and the picker shows a busy state; the latest selection wins, stale results never switch the view, a failure falls back with a visible message, and selecting the layer again retries. `main.rs` only runs the fetches it asks for.
- **`src/search.rs`** and **`src/search_ui.rs`**: offline place search. `search.rs` is browser-free: it parses `data/places.tsv`, folds case and accents (`zurich` finds `Zürich`), and ranks exact, then prefix, then word-prefix, then substring matches, with population as the tiebreak. `search_ui.rs` wires the search box and results list, and fetches `places.tsv` the first time the box gets focus, not at startup.
- **`src/labels.rs`** and **`src/label_layer.rs`**: place labels for countries, major cities, oceans and seas. `labels.rs` is browser-free. Each frame the view changes it picks candidates (a per-kind, per-tier altitude gate so density grows with zoom; countries, oceans and seas must also be big enough on screen to hold their name), drops anything on or past the horizon (labels fade out near the limb), orders them by priority (oceans, countries, then cities by population tier, with labels already visible winning ties so they don't flicker), and places them greedily, skipping any whose box overlaps one already placed (at most 90). Anchors go through `Camera::project`. `label_layer.rs` measures text once, fades labels in and out (instantly when `prefers-reduced-motion` is set) and paints them on the `#labels` canvas, which has `pointer-events: none` so hover, click and drag reach the globe. It does nothing on idle frames. Labels sit under the HUD, search box and panels, so those cover them.
- **`src/controls.rs`**: a browser-free input state machine. It takes plain events (pointer down/move/up, wheel, tick, zoom, fly-to, reset, key nudges, rotate, north-up) and mutates a `Camera`: drag, pinch, two-finger twist, alt-drag rotation, fling inertia, zoom anchoring and flights (which also turn the bearing). It is unit tested natively.
- **`src/main.rs`**: wires up the DOM, translates input events (pointer, wheel, keyboard) into `controls::Event`s and runs the animation loop, which only redraws when something changes.

## Data

Map data is from [Natural Earth](https://www.naturalearthdata.com/) (1:50m land, lakes, admin-0 boundary lines and admin-0 countries, whose names, ISO codes, continent, subregion and population estimate feed the info panel, plus 1:10m populated places for search and labels, and 1:50m marine polygons for ocean and sea names). It is in the public domain.

The GeoJSON sources live in `data-src/`. The app downloads compact binary files (`assets/data/*.bin`, quantized to 1e-4° and delta/varint encoded, about 680 KB instead of 6.4 MB; the country file is about 290 KB, fetched after the first frame). After changing a source file, regenerate them with `python3 scripts/convert_geojson.py`; `scripts/check-data.sh` (CI job "Data") checks byte-for-byte that the committed files are what the script generates.

Place search uses `assets/data/places.tsv` (about 350 KB, about 135 KB gzipped), generated by the same script from the 1:50m countries and 1:10m populated places in `data-src/`. It holds name, country, coordinates, population (for ranking), the size to frame for countries, and alternate names. It is downloaded only when the search box is first focused.

Labels use `assets/data/labels.tsv` (about 125 KB, about 55 KB gzipped): country label points, populated places with at least 100,000 people plus all national capitals, and oceans and seas, each with a tier that sets how far in it appears. It is generated by `convert_geojson.py` and fetched right after the first frame. Ocean and sea label points come from Natural Earth's `ne_50m_geography_marine_polys`: `scripts/marine_label_points.py` picks one point inside each polygon's largest ring and writes `data-src/ne_50m_marine_label_points.geojson`. Re-run it only when updating that source.
