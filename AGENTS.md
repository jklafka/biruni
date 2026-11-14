# AGENTS.md

These are the guidelines for AI coding agents (and humans) working in this repository.

## Project

`maps` is a web app for exploring maps. It is written in Rust, compiled to WebAssembly, and rendered with WebGL2. There is no JavaScript framework and no backend. The build output is a static site.

| Path | Purpose |
| --- | --- |
| `src/main.rs` | Entry point: DOM wiring, input events, the animation loop, URL-hash view state |
| `src/camera.rs` | Orbit camera math: picking, projection, panning, zoom anchoring |
| `src/view.rs` | HUD text formatting and the `#lat,lon,alt_km` URL hash |
| `src/geo.rs` | Parses GeoJSON (`parse_shapes`) and rasterizes it into the map texture with Canvas 2D |
| `src/render.rs` | WebGL2 setup, shader compilation, texture upload, draw call |
| `src/shaders/` | GLSL ES 3.00. `globe.frag` ray-traces the globe for each pixel. |
| `assets/data/` | Natural Earth GeoJSON (public domain), copied to `dist/data/` by Trunk |
| `.github/workflows/ci.yml` | CI: format, clippy, tests, shader validation, release build |
| `index.html` | Page shell, HUD and controls. Trunk reads it as the build entry. |

## Setup and commands

You need rustup with the `wasm32-unknown-unknown` target, plus [Trunk](https://trunkrs.dev).

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
- The PR description says what changed, why, and how it was verified. Include a screenshot for visual changes.
- Do not add attribution lines to commits or PR descriptions: no `Co-Authored-By` trailers and no "Generated with" footers.
- Before opening a PR, make sure every check in **Before you open a PR** below passes.
- Never commit `dist/` or `target/`. Never commit secrets or API keys.

## Before you open a PR

CI (`.github/workflows/ci.yml`) runs the checks below on every pull request. Branch protection on `main` requires all of them to pass, plus a PR, before merging. Run them locally first:

1. **Format:** `cargo fmt --all --check`
2. **Clippy:** `cargo clippy --target wasm32-unknown-unknown -- -D warnings` and `cargo clippy --all-targets -- -D warnings`
3. **Test:** `cargo test --all-targets`
4. **Shaders:** `glslangValidator src/shaders/*.vert src/shaders/*.frag` (from `brew install glslang` or `apt install glslang-tools`)
5. **Build:** `trunk build --release`
6. For rendering or interaction changes, look at the result in a browser. The URL hash sets the view, so you can test a specific place reproducibly, e.g. `http://127.0.0.1:8080/#46.8,8.2,600` (lat, lon, altitude in km). Check at least three views: the whole globe, a mid zoom, and the closest zoom (about 400 km). If you can't look at it, say so in the PR. Don't claim it works.

Don't weaken or skip a check to get a PR through. Fix the cause. If a check really is wrong, change it in its own PR and explain why.

## Testing

- Tests run natively (`cargo test`), not in wasm, so tested code must not call browser APIs. Put pure logic in browser-free functions:
  - `camera.rs`: projection, picking and panning math
  - `view.rs`: HUD text and URL-hash parsing
  - `geo::parse_shapes`: GeoJSON → shapes
- Every bug fix comes with a test that fails without the fix.
- New math or parsing comes with tests, including edge cases: poles, the ±180° seam, NaN/∞, malformed input.
- `render.rs` checks that every uniform Rust looks up is declared in `globe.frag`. When you add a uniform, add it to that list.
- `geo.rs` checks that the bundled datasets parse and stay within lon/lat bounds.

## Code guidelines

### Rust

- Target stable Rust, edition 2021. Keep the dependency list small. Before adding a crate, make sure it compiles to `wasm32-unknown-unknown` and is worth the added wasm size. The release wasm is currently about 140 KB.
- Add new `web-sys` APIs by enabling the matching feature in `Cargo.toml`.
- Don't use `unwrap()` on anything that can fail at runtime in the browser (DOM lookups, fetches, WebGL calls). Return `Result<_, JsValue>` and surface errors to the user, as `start()` does with the `#loading` panel.
- Keep pure math, such as camera and projection code, free of `web-sys`, so it can be unit tested natively. New math gets tests.
- Event listeners are added with the `listen` helper and live for the whole page (`Closure::forget`). Don't leak closures in code paths that run repeatedly.

### Coordinate conventions (must stay in sync)

`src/camera.rs` and `src/shaders/globe.frag` implement the same math and must agree exactly:

- World space: +Y is the north pole, longitude 0° lies on +Z, and 90°E lies on +X. The globe has radius 1.
- Camera space: the camera sits at `(0, 0, 1 + alt)` looking down −Z, with north up on screen.
- The field of view (40°) spans the **shorter** viewport axis.
- The texture is equirectangular: `u = lon/360 + 0.5`, `v = 0.5 − lat/180`, with row 0 at the north.

If you change one side, change the other in the same PR, and extend the round-trip tests in `camera.rs`.

### Shaders

- Write GLSL ES 3.00 for WebGL2. Don't rely on extensions without a fallback.
- Don't return early in the fragment shader before `fwidth`, `dFdx` or `dFdy`, or before implicit-LOD `texture()` calls. Derivatives need every pixel in the 2×2 quad to follow the same path.
- Keep line widths, such as the graticule, coast and borders, constant on screen by scaling with `fwidth`, not by world units.
- Watch the longitude seam at ±180°. Mipmapped lookups use the two-parameterization trick (`u1`/`u2`) to avoid a visible seam.

### Rendering and performance

- Only redraw when something changed. The frame loop sets `dirty`. Don't add work that runs every frame while the app is idle.
- The map texture is 8192×4096 RG8 with mipmaps (about 85 MB of GPU memory), falling back to 4096 on GPUs that can't handle it. Don't increase it without a plan for mobile.
- Rasterize on canvases of 4096×4096 or smaller, because some mobile browsers cap a canvas at 16.7 MP. That's why the map is drawn in two halves.
- Debounce `history.replaceState` (see `hash_stale` and `last_hash`), because Safari throttles it.

### UI

- Every interaction needs a mouse, touch and keyboard path where reasonable. Keep the controls table in `README.md` up to date.
- Keep the UI usable at phone widths. The canvas uses `touch-action: none`, and gestures are handled in Rust.

## Data

- Map data comes from [Natural Earth](https://www.naturalearthdata.com/) (1:50m), which is public domain. If you add another dataset, check that its license allows redistribution, and credit it in `README.md`.
- Keep large data out of the wasm binary. Fetch it at runtime from `assets/` instead of using `include_str!`/`include_bytes!`.
- If you change data files or how they are loaded, note the download size in the PR.

## Documentation

- Update `README.md` when you change commands, controls or architecture.
- Update this file when you change a convention it describes.
