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

`visual-tests/` screenshots the globe in headless Chromium (software WebGL2 through SwiftShader) at five fixed views set by the URL hash: the whole globe, mid zoom, closest zoom, the antimeridian and the monochrome layer. It waits for the map data to load, then compares each screenshot with a committed baseline in `visual-tests/baselines/` using pixelmatch. A view fails if more than 0.5% of its pixels differ noticeably. This tolerates small rasterizer differences but catches a flipped land mask, a black globe or a shifted camera.

```sh
trunk build --release
cd visual-tests
npm ci && npx playwright install chromium   # first time only
npm test                                    # compare with baselines
```

New screenshots go to `visual-tests/actual/` and diff images to `visual-tests/diff/` (both git-ignored). CI uploads them as the `visual-screenshots` artifact.

**Updating baselines.** Rendering differs slightly between macOS and Linux, so baselines are generated on Linux, the same as CI. After an intentional rendering change, push the branch, download the `visual-screenshots` artifact from the CI run (`gh run download <run-id> -n visual-screenshots`), copy `actual/*.png` into `visual-tests/baselines/`, look at them, and commit. `npm run update` rewrites the baselines locally, which is fine for experiments, but don't commit macOS-generated baselines.

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
| Switch map layer | layer picker (top right; under the title on phones) | layer picker | `1`, `2`, or `L` to cycle |
| Share a link to the current view | share button (bottom right) | share button (opens the share sheet where supported, else copies) | `S` |
| Switch scale bar between km and mi | click the scale bar | tap the scale bar | `U` |

The HUD shows a scale bar (a round 1/2/5 × 10ⁿ distance) for the ground distance at the screen center. It is hidden when you are zoomed out far enough (above about 1 globe radius of altitude) that the scale varies too much across the view.

The current view is saved in the URL hash as `#lat,lon,altitude_km`, so you can link straight to a place, e.g. `#46.8,8.2,600`. A non-default layer is added as `&layer=<id>`, e.g. `#46.8,8.2,600&layer=mono`. Switching layers never moves the camera.

## How it works

- **`src/camera.rs`**: an orbit camera that keeps north up. It handles picking (screen point → lat/lon), projection (lat/lon → screen point), and panning that makes the surface follow your cursor. When you zoom, the point under the cursor stays where it is.
- **`src/geo.rs`**: fetches the compact binary layers (converted from [Natural Earth](https://www.naturalearthdata.com/) GeoJSON) and rasterizes them with Canvas 2D into an 8192×4096 equirectangular texture. The red channel holds land with lakes cut out; the green channel holds country borders.
- **`src/render.rs`** and **`src/shaders/`**: WebGL2 draws one full-screen triangle. The fragment shader ray-traces the sphere for each pixel, converts the hit point to lat/lon, and samples the texture. Close up, it thresholds the mask so coastlines stay crisp. It also adds the graticule, lighting and atmosphere.
- **`src/view.rs`**: formats the HUD text, picks the scale bar length, parses the URL hash, and defines the `Layer` enum. A layer is a shader palette selected by the `u_layer` uniform; layers that need their own data should lazy-load it on first selection.
- **`src/main.rs`**: wires up the DOM, input events (pointer, wheel, keyboard) and the animation loop. The loop gives zoom easing, fly-to and pan inertia, and only redraws when something changes.

## Data

Map data is from [Natural Earth](https://www.naturalearthdata.com/) (1:50m land, lakes and admin-0 boundary lines). It is in the public domain.

The GeoJSON sources live in `data-src/`. The app downloads compact binary files (`assets/data/*.bin`, quantized to 1e-4° and delta/varint encoded, about 390 KB instead of 3.3 MB). After changing a source file, regenerate them with `python3 scripts/convert_geojson.py`; a test checks the committed files match the GeoJSON.
