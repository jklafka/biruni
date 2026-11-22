# Development

How to build, test and debug Biruni. Contribution rules are in [AGENTS.md](../AGENTS.md).

## Running locally

Prerequisites:

- Rust via [rustup](https://rustup.rs) with the wasm target: `rustup target add wasm32-unknown-unknown`
- [Trunk](https://trunkrs.dev): `brew install trunk` or `cargo install trunk`
- Python 3 (standard library only): Trunk runs `scripts/hash_data.py` after every build

```sh
trunk serve --open      # dev server with live reload at http://127.0.0.1:8080
trunk build --release   # optimized static site in dist/
cargo test              # unit tests (run natively)
```

> If `rustup` and Homebrew's `rust` are both installed, make sure rustup's toolchain comes first on your `PATH`. Homebrew's `rust` has no `wasm32` target.

Every pull request runs CI (format, clippy, tests, shader validation, a release build and visual regression tests). `main` only accepts pull requests that pass all of it. See [AGENTS.md](../AGENTS.md) for contribution guidelines.

## Visual regression tests

`visual-tests/` screenshots the map in headless Chromium (software WebGL2 through SwiftShader) at fixed views set by the URL hash: the whole globe, mid zoom, closest zoom, the antimeridian, the poles, rotated views, the monochrome layer, the thematic layers with their legends (population on the globe, GDP per capita in Robinson, income group at phone width), the satellite layer (globe, mid zoom, closest zoom, Robinson, day/night; each waits for the image to download and upload), the day/night terminator in both layers (with the sun pinned by `&t=`), and the four flat projections (the whole world in each, a zoomed Mercator and Robinson across the seam). It waits for the map data to load, then compares each screenshot with a committed baseline in `visual-tests/baselines/` using pixelmatch. A view fails if more than 0.5% of its pixels differ noticeably. This tolerates small rasterizer differences but catches a flipped land mask, a black globe or a shifted camera.

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

It needs `gh`. It finds the latest completed CI run for the PR's head commit, downloads the `visual-screenshots` artifact, copies `actual/*.png` into `visual-tests/baselines/` and commits it as you. A PNG is copied only if it is new or differs from the committed baseline: the size changed, or at least one pixel differs at the visual test's per-pixel threshold (`visual-tests/same-pixels.mjs`, using `PIXEL_THRESHOLD` from `visual-tests/thresholds.mjs`). The 0.5% allowance the test uses is not applied, so even a small intentional change is picked up, while byte-level re-encoding of unchanged views is ignored. Each file is reported as `changed (N px differ)`, `new` or `unchanged pixels, skipped`, and nothing is committed if no file qualifies. Pass `--all` to copy every PNG regardless, for a full refresh. It runs `npm ci` in `visual-tests/` if `node_modules` is missing. `scripts/update-baselines.sh --run <CI run id>` does the same copy from any run's artifact, with no PR checks and no commit, which is handy for inspecting a run (`git status` shows the result). It refuses to run if the current branch isn't the PR's branch or the tree is dirty, and it says so if CI has no artifact yet. Look at the new PNGs before you push, then wait for CI to pass on the new commit. The commit is made by you, not a bot, so it doesn't trip the repository's extra-approval rule for unattributed changes. `npm run update` rewrites the baselines locally, which is fine for experiments, but don't commit macOS-generated baselines.

## Debug aids

These are for testing, not controls, and have no UI. `?debug=ctxloss` (before the `#` hash) makes the backquote key (`` ` ``) lose the WebGL context through the `WEBGL_lose_context` extension; press it again to restore it. This exercises context recovery (everything is rebuilt from data kept in memory, with no refetch) without the browser console. Without the flag the key does nothing. `?tex=low|high` forces a texture tier.
