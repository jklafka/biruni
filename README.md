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

`visual-tests/` screenshots the globe in headless Chromium (software WebGL2 through SwiftShader) at seven fixed views set by the URL hash: the whole globe, mid zoom, closest zoom, the antimeridian, the monochrome layer, and the day/night terminator in both layers (with the sun pinned by `&t=`). It waits for the map data to load, then compares each screenshot with a committed baseline in `visual-tests/baselines/` using pixelmatch. A view fails if more than 0.5% of its pixels differ noticeably. This tolerates small rasterizer differences but catches a flipped land mask, a black globe or a shifted camera.

```sh
trunk build --release
cd visual-tests
npm ci && npx playwright install chromium   # first time only
npm test                                    # compare with baselines
```

New screenshots go to `visual-tests/actual/` and diff images to `visual-tests/diff/` (both git-ignored). CI uploads them as the `visual-screenshots` artifact.

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
| Rotate | drag | one-finger drag | arrow keys |
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

Hovering a country (desktop) highlights it and shows its name in the HUD. Clicking or tapping a country opens a panel with its flag, name, continent, region, population and ISO codes; clicking the ocean, the × button or `Esc` closes it. Hover and click are ignored until the country data has loaded, which happens after the first frame. Tiny countries that are barely visible at the current zoom are hard to hit; zoom in.

The HUD shows a scale bar (a round 1/2/5 × 10ⁿ distance) for the ground distance at the screen center. It is hidden when you are zoomed out far enough (above about 1 globe radius of altitude) that the scale varies too much across the view.

The current view is saved in the URL hash as `#lat,lon,altitude_km`, so you can link straight to a place, e.g. `#46.8,8.2,600`. A non-default layer is added as `&layer=<id>`, e.g. `#46.8,8.2,600&layer=mono`. Switching layers never moves the camera.

The day/night terminator is off by default. `&sun=on` turns it on at load, and `&t=<unix seconds>` pins the sun to a fixed time instead of the clock (the visual tests use this so screenshots are deterministic). These two parameters are read at load only; the app does not write them back into the hash. While on, the sun follows the clock and is refreshed once a minute by a timer. That timer is the only work the app does while idle, and it only redraws when the terminator is on.

## How it works

- **`src/camera.rs`**: an orbit camera that keeps north up. It handles picking (screen point → lat/lon), projection (lat/lon → screen point), and panning that makes the surface follow your cursor. When you zoom, the point under the cursor stays where it is.
- **`src/geo.rs`**: fetches the compact binary layers (converted from [Natural Earth](https://www.naturalearthdata.com/) GeoJSON) and rasterizes them with Canvas 2D into an equirectangular texture, 8192×4096 on capable desktops and 4096×2048 on touch, low-memory (`navigator.deviceMemory` of 4 GB or less) and 4096-limited GPUs. Append `?tex=low` or `?tex=high` to the URL to force a tier. The red channel holds land with lakes cut out; the green channel holds country borders.
- **`src/country.rs`**: decodes the country polygons and attribute table (also lazy-loaded), hit-tests a lat/lon against them (bounding-box prefilter, then even-odd point-in-polygon over outer rings and holes), and rasterizes a country ID per texel (`index + 1`, 4096×2048, one byte each) with a scanline fill. `src/info_panel.rs` fills the panel in `index.html`.
- **`src/render.rs`** and **`src/shaders/`**: WebGL2 draws one full-screen triangle. The fragment shader ray-traces the sphere for each pixel, converts the hit point to lat/lon, and samples the texture. Close up, it thresholds the mask so coastlines stay crisp. It also adds the graticule, lighting and atmosphere. The country-ID raster is a second `R8` texture (4096 wide, 8 MB, or 2048 wide, 2 MB on the small tier); the shader tints the texels whose ID equals `u_highlight`, bilinearly blended so the edge stays smooth. Hover only changes that uniform, so it redraws once per country change, not per mouse move.
- **`src/sun.rs`**: low-precision solar position (declination, plus equation of time via right ascension minus sidereal time) giving the subsolar point and the world-space sun direction for the `u_sun` uniform. Accurate to about 0.01°.
- **`src/view.rs`**: formats the HUD text, picks the scale bar length, parses the URL hash, and defines the `Layer` enum. A layer is a shader palette selected by the `u_layer` uniform; layers that need their own data should lazy-load it on first selection.
- **`src/search.rs`** and **`src/search_ui.rs`**: offline place search. `search.rs` is browser-free: it parses `data/places.tsv`, folds case and accents (`zurich` finds `Zürich`), and ranks exact, then prefix, then word-prefix, then substring matches, with population as the tiebreak. `search_ui.rs` wires the search box and results list, and fetches `places.tsv` the first time the box gets focus, not at startup.
- **`src/main.rs`**: wires up the DOM, input events (pointer, wheel, keyboard) and the animation loop. The loop gives zoom easing, fly-to and pan inertia, and only redraws when something changes.

## Data

Map data is from [Natural Earth](https://www.naturalearthdata.com/) (1:50m land, lakes, admin-0 boundary lines and admin-0 countries, whose names, ISO codes, continent, subregion and population estimate feed the info panel, plus 1:10m populated places for search). It is in the public domain.

The GeoJSON sources live in `data-src/`. The app downloads compact binary files (`assets/data/*.bin`, quantized to 1e-4° and delta/varint encoded, about 680 KB instead of 6.4 MB; the country file is about 290 KB, fetched after the first frame). After changing a source file, regenerate them with `python3 scripts/convert_geojson.py`; a test checks the committed files match the GeoJSON.

Place search uses `assets/data/places.tsv` (about 350 KB, about 135 KB gzipped), generated by the same script from the 1:50m countries and 1:10m populated places in `data-src/`. It holds name, country, coordinates, population (for ranking), the size to frame for countries, and alternate names. It is downloaded only when the search box is first focused.
