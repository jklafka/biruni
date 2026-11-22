# Deployment

How the site and its data are published.

The site is hosted on [Cloudflare Pages](https://pages.cloudflare.com). Cloudflare's build image has no Rust toolchain, so `.github/workflows/deploy.yml` builds `dist/` in GitHub Actions and uploads it with `wrangler pages deploy`. Pushes to `main` publish to production. Pull requests from this repository get a preview deployment, and the preview URL is posted as a PR comment. Fork PRs are not deployed. If the secrets below are missing, the deploy step is skipped and the workflow still passes. `assets/_headers` sets the MIME type and cache headers.

## Map data on R2

On pushes to `main` the map data is served from a Cloudflare R2 bucket (`biruni-data`, public at `https://biruni-data.jklafka.cloud`) instead of from Pages. After the build, the deploy job (only if the `CLOUDFLARE_R2_TOKEN` secret is set; otherwise the data stays on Pages):

1. validates `dist/data` against the data currently live (`scripts/validate_data.py --baseline`, the baseline fetched by `scripts/publish_data.py fetch-baseline`),
2. sets the bucket's CORS rules from `scripts/r2-cors.json` (GET and HEAD from any origin: the data is public and needs no credentials, and R2 cannot match the per-deploy preview subdomains `<hash>.biruni.pages.dev` by wildcard),
3. uploads each content-hashed file that the bucket lacks (`publish_data.py plan`, then `wrangler r2 object put`, immutable cache headers) and checks that each one is served with the right size and CORS header,
4. rewrites `dist/data/manifest.json` to absolute R2 URLs, drops `dist/data/files`, and only then runs `pages deploy`.

Any failure stops the job before the Pages deploy, so a manifest never points at missing files. The manifest ships with the app, so the app and its data switch together. Hashed files are never overwritten, and old ones are never deleted (for now), so pages that are still open keep working. Pull requests, `trunk serve` and the visual tests keep using the relative local data.

## Spacecraft and small bodies (weekly refresh)

`assets/data/space.bin` holds spacecraft positions (sampled from 30 days before to a year after the fetch) and orbital elements of dwarf planets, asteroids and comets, including the few brightest comets near perihelion at the time. It comes from NASA JPL's Horizons and Small-Body Database APIs through `scripts/fetch_space.py`; the browser never calls JPL. The deploy workflow runs every Monday (05:23 UTC) to fetch it fresh and deploy `main` with it; run it by hand with **Actions → Deploy → Run workflow → refresh_space**. Ordinary pushes to `main` keep the live file if it is newer than the committed one, which is a fixed snapshot (as of 2026-11-20) used by development, CI and the visual tests. Refetch the snapshot with `python3 scripts/fetch_space.py assets/data/space.bin --date YYYY-MM-DD` (about 35 requests, a second apart).

## Tile packs

The image layers get sharper at close zoom from **tile packs**: one file per layer holding a pyramid of 256×256 WebP tiles, read with HTTP range requests (design and format: [`tile-streaming.md`](tile-streaming.md)). Packs live only on R2, not in git or on Pages. To build and publish them, run the **Publish tiles** workflow (Actions → Publish tiles → Run workflow, or `gh workflow run publish-tiles.yml`). It builds each pack from the pinned sources with `scripts/make_tiles.py`, uploads the ones R2 lacks, checks they are served with range support and CORS (`publish_data.py check-tilesets`), and attaches the updated `assets/data/tilesets.json`. Commit that file in a pull request. The deploy checks again that every listed pack is served. Locally, `python3 scripts/make_tiles.py build <layer>` writes a pack to `.data-cache/tiles/` (needs Pillow, and NumPy for terrain). Satellite and Night lights go to level 7 (about 0.6 km per texel) from NASA's 500 m mosaics: the build downloads eight pinned source tiles per layer (about 420 MB and 250 MB), takes about a minute and peaks near 7 GB of memory. The app reads a pack's directory in 32 KB pages as the view needs them.

Cloudflare's cache may keep the first response it sees for an R2 file, including its CORS headers, so the scripts send `Origin: https://biruni.jklafka.cloud` on every request. Do the same when checking R2 files by hand (`curl -H 'Origin: https://biruni.jklafka.cloud' ...`).

One-time setup for the repository owner (done): the R2 bucket `biruni-data`; its custom domain `biruni-data.jklafka.cloud`; the zone setting Browser Cache TTL = **Respect Existing Headers** (so R2's `Cache-Control` is kept); and the repository secret `CLOUDFLARE_R2_TOKEN`, an API token with **Account, Workers R2 Storage, Edit**. The R2 steps use that token, not the Pages one.

One-time setup for the repository owner:

1. In the Cloudflare dashboard, go to Workers & Pages, then create a **Pages** project named `biruni` using **Direct Upload** (any placeholder upload). Set its production branch to `main`. A different name needs a matching change to `PAGES_PROJECT` in the workflow.
2. Create an API token (My Profile, API Tokens) with the permission **Account, Cloudflare Pages, Edit**.
3. Copy your Account ID from the dashboard.
4. In GitHub, under Settings, Secrets and variables, Actions, add the repository secrets `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`.
5. Optionally, add the **Deploy to Cloudflare Pages** job as a required check in the `main` branch protection.
6. Attach the custom domain (`biruni.jklafka.cloud`) under the project's Custom domains. On the zone, set Browser Cache TTL to **Respect Existing Headers** (or add a Cache Rule for this hostname that does), or Cloudflare raises short `max-age` values such as `raster-worker.js`'s to the zone default.
