// Visual regression test: screenshots fixed views of the globe in headless
// Chromium (software WebGL2 via SwiftShader) and compares them to baselines.
//
//   node run.mjs            compare against baselines/ (exit 1 on mismatch)
//   node run.mjs --update   overwrite baselines/ with the new screenshots
//
// Expects a built site in ../dist (trunk build --release). Set BASE_URL to
// test an already-running server instead.
import { chromium } from "playwright";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";
import { PIXEL_THRESHOLD, MAX_DIFF_FRACTION } from "./thresholds.mjs";
import { createServer } from "node:http";
import { readFile, mkdir, writeFile, rm } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const dist = join(here, "..", "dist");
const update = process.argv.includes("--update");

// PIXEL_THRESHOLD and MAX_DIFF_FRACTION live in thresholds.mjs.

const VIEWPORT = { width: 800, height: 600 };
const VIEWS = [
  { name: "globe", hash: "" },
  { name: "mid-zoom", hash: "#38.5,22.5,1500" },
  // Same view as mid-zoom, but the WebGL context is lost and restored through
  // the ?debug=ctxloss key before the screenshot. It must match mid-zoom, and
  // recovery must not start a Worker or download anything again.
  { name: "ctxloss-restored", hash: "#38.5,22.5,1500", query: "?debug=ctxloss", loseContext: true },
  { name: "close-zoom", hash: "#47,8.5,400" },
  { name: "antimeridian", hash: "#-10,175,9000" },
  { name: "phone", hash: "#47,8.5,1500", viewport: { width: 390, height: 844 } },
  // Free rotation: both poles centered exactly, and arbitrary bearings
  // (the 4th hash field), at globe, mid and closest zoom.
  { name: "north-pole", hash: "#90,0,6000" },
  { name: "south-pole", hash: "#-90,0,6000" },
  { name: "rotated", hash: "#47,8.5,1500,135" },
  { name: "rotated-close", hash: "#47,8.5,400,250" },
  // Flat projections: the whole world in each, then closer views and the
  // seam. The camera center (lat, lon) is the middle row and central meridian.
  { name: "proj-equirect", hash: "#0,0,19000&proj=equirect" },
  { name: "proj-mercator", hash: "#10,10,19000&proj=mercator" },
  { name: "proj-robinson", hash: "#0,0,19000&proj=robinson" },
  { name: "proj-mollweide", hash: "#0,0,19000&proj=mollweide" },
  { name: "proj-mercator-zoom", hash: "#47,8.5,1500&proj=mercator" },
  { name: "proj-robinson-seam", hash: "#-10,175,19000&proj=robinson" },
  { name: "mono-mid-zoom", hash: "#38.5,22.5,1500&layer=mono" },
  // Satellite imagery (NASA Blue Marble): fetched on first selection from
  // data/blue_marble.webp, so these views wait for the layer to be shown.
  { name: "imagery-globe", hash: "#25,10,15000&layer=imagery" },
  { name: "imagery-mid-zoom", hash: "#38.5,22.5,1500&layer=imagery" },
  { name: "imagery-close-zoom", hash: "#47,8.5,400&layer=imagery" },
  { name: "imagery-robinson", hash: "#0,0,19000&proj=robinson&layer=imagery" },
  { name: "imagery-day-night", hash: "#30,0,9000&layer=imagery&sun=on&t=1718992800" },
  // The sun is pinned with &t=<unix seconds> so these never depend on the
  // clock: 2024-06-21 18:00 UTC, sun over about 90 W.
  { name: "day-night", hash: "#30,0,9000&sun=on&t=1718992800" },
  { name: "mono-day-night", hash: "#30,0,9000&layer=mono&sun=on&t=1718992800" },
];

const shots = {};

const MIME = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".geojson": "application/json",
  ".css": "text/css",
  ".png": "image/png",
  ".webp": "image/webp",
  ".svg": "image/svg+xml",
};

function serve() {
  const server = createServer(async (req, res) => {
    try {
      let path = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname));
      if (path.endsWith("/")) path += "index.html";
      const file = join(dist, path);
      if (!file.startsWith(dist)) throw new Error("outside dist");
      const body = await readFile(file);
      res.writeHead(200, { "content-type": MIME[extname(file)] ?? "application/octet-stream" });
      res.end(body);
    } catch {
      res.writeHead(404).end("not found");
    }
  });
  return new Promise((resolve) =>
    server.listen(0, "127.0.0.1", () =>
      resolve({ server, url: `http://127.0.0.1:${server.address().port}/` }),
    ),
  );
}

const baselineDir = join(here, "baselines");
const actualDir = join(here, "actual");
const diffDir = join(here, "diff");
await rm(actualDir, { recursive: true, force: true });
await rm(diffDir, { recursive: true, force: true });
await mkdir(actualDir, { recursive: true });
await mkdir(diffDir, { recursive: true });
await mkdir(baselineDir, { recursive: true });

let server;
let base = process.env.BASE_URL;
if (!base) {
  if (!existsSync(join(dist, "index.html"))) {
    console.error("dist/index.html not found. Run `trunk build --release` first.");
    process.exit(2);
  }
  ({ server, url: base } = await serve());
}

const browser = await chromium.launch({
  args: [
    "--use-angle=swiftshader",
    "--use-gl=angle",
    "--enable-unsafe-swiftshader",
    "--ignore-gpu-blocklist",
  ],
});

let failed = false;
try {
  for (const view of VIEWS) {
    // Reduced motion makes label fades instant, so the screenshot is stable.
    const page = await browser.newPage({
      viewport: view.viewport ?? VIEWPORT,
      deviceScaleFactor: 1,
      reducedMotion: "reduce",
    });
    const problems = [];
    page.on("pageerror", (e) => problems.push(String(e)));
    page.on("console", (m) => m.type() === "error" && problems.push(m.text()));

    // Park the virtual mouse outside the viewport. Chromium replays a
    // synthetic mouse move at the last known position after layout changes,
    // and that position is (0, 0) if the mouse never moved. (0, 0) lands on
    // the globe and would hover-highlight a country.
    await page.mouse.move(-20, -20);
    const requests = [];
    let workers = 0;
    page.on("request", (r) => requests.push(r.url()));
    page.on("worker", () => workers++);
    await page.goto(base + (view.query ?? "") + view.hash);
    // The app hides #loading once the map texture is uploaded (or marks it
    // .error on failure). Then wait for two animation frames so the redraw
    // triggered by the upload has reached the screen.
    await page.waitForSelector("#loading.hidden, #loading.error", {
      state: "attached",
      timeout: 60_000,
    });
    if (await page.locator("#loading.error").count()) {
      throw new Error(`${view.name}: map failed to load: ${await page.locator("#loading").innerText()}`);
    }
    // Labels load after the map; the app flags the canvas once they are drawn.
    await page.waitForSelector("#labels[data-ready]", { state: "attached", timeout: 30_000 });
    // Country data (hover, political fills) loads last; the app flags the canvas.
    await page.waitForSelector("#globe[data-countries]", { state: "attached", timeout: 30_000 });
    // A data layer is shown only once its file is fetched, decoded and
    // uploaded. The app sets data-layer on the canvas then, and fails loudly
    // (status text) if the download or decode fails.
    const wantLayer = /[&#]layer=([a-z]+)/.exec(view.hash)?.[1];
    if (wantLayer === "imagery") {
      await page.waitForSelector(`#globe[data-layer="${wantLayer}"][data-imagery="1"]`, {
        state: "attached",
        timeout: 60_000,
      });
    }
    await page.evaluate(
      () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))),
    );
    if (view.loseContext) {
      const before = { requests: requests.length, workers };
      const lost = () =>
        page.evaluate(() => document.getElementById("globe").getContext("webgl2").isContextLost());
      await page.focus("#globe");
      await page.keyboard.press("`"); // debug key: lose the context
      await page.waitForFunction(() => document.getElementById("globe").getContext("webgl2").isContextLost());
      await page.waitForSelector("#loading:not(.hidden)", { state: "attached", timeout: 10_000 });
      await page.keyboard.press("`"); // and restore it
      await page.waitForFunction(() => !document.getElementById("globe").getContext("webgl2").isContextLost());
      await page.waitForSelector("#loading.hidden, #loading.error", { state: "attached", timeout: 30_000 });
      if (await page.locator("#loading.error").count() || (await lost())) {
        throw new Error(`${view.name}: context was not restored`);
      }
      // Let the re-render reach the screen.
      await page.evaluate(
        () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))),
      );
      await page.waitForTimeout(1000);
      if (requests.length !== before.requests || workers !== before.workers) {
        console.error(
          `FAIL ${view.name}: recovery re-downloaded or started a worker: ${requests.slice(before.requests)} workers +${workers - before.workers}`,
        );
        failed = true;
      }
    }
    // No pointer may be over the globe: hover would tint a country and show
    // a cursor readout. Fail loudly instead of comparing a tinted frame.
    const cursor = (await page.locator("#hud-cursor").innerText()).trim();
    if (cursor !== "\u2014") {
      console.error(`FAIL ${view.name}: pointer over the globe (HUD cursor "${cursor}")`);
      failed = true;
    }
    const shot = await page.screenshot({ animations: "disabled" });
    await page.close();

    const actual = PNG.sync.read(shot);
    // Guard against a blank canvas: a rendered globe has many distinct colours.
    const colours = new Set();
    for (let i = 0; i < actual.data.length; i += 4 * 97) {
      colours.add((actual.data[i] << 16) | (actual.data[i + 1] << 8) | actual.data[i + 2]);
    }
    if (colours.size < 20) {
      console.error(`FAIL ${view.name}: screenshot looks blank (${colours.size} distinct colours)`);
      failed = true;
    }
    if (problems.length) console.warn(`${view.name}: page errors:\n  ${problems.join("\n  ")}`);

    shots[view.name] = actual;
    if (view.loseContext) {
      // Recovery must reproduce the no-loss render, not just match a baseline.
      const ref = shots["mid-zoom"];
      const d = new PNG({ width: actual.width, height: actual.height });
      const n = pixelmatch(ref.data, actual.data, d.data, actual.width, actual.height, {
        threshold: PIXEL_THRESHOLD,
      });
      const ok = n <= actual.width * actual.height * 0.0005;
      console.log(`${ok ? "ok  " : "FAIL"} ${view.name} vs mid-zoom (no loss): ${n} px differ`);
      if (!ok) failed = true;
    }
    await writeFile(join(actualDir, `${view.name}.png`), shot);
    const baselinePath = join(baselineDir, `${view.name}.png`);

    if (update) {
      await writeFile(baselinePath, shot);
      console.log(`updated baselines/${view.name}.png`);
      continue;
    }
    if (!existsSync(baselinePath)) {
      console.error(`FAIL ${view.name}: no baseline (run \`npm run update\`)`);
      failed = true;
      continue;
    }
    const expected = PNG.sync.read(await readFile(baselinePath));
    if (expected.width !== actual.width || expected.height !== actual.height) {
      console.error(
        `FAIL ${view.name}: size ${actual.width}x${actual.height}, baseline ${expected.width}x${expected.height}`,
      );
      failed = true;
      continue;
    }
    const diff = new PNG({ width: actual.width, height: actual.height });
    const n = pixelmatch(expected.data, actual.data, diff.data, actual.width, actual.height, {
      threshold: PIXEL_THRESHOLD,
    });
    const frac = n / (actual.width * actual.height);
    const ok = frac <= MAX_DIFF_FRACTION;
    console.log(
      `${ok ? "ok  " : "FAIL"} ${view.name}: ${n} px differ (${(frac * 100).toFixed(3)}%, limit ${MAX_DIFF_FRACTION * 100}%)`,
    );
    if (!ok) {
      failed = true;
      await writeFile(join(diffDir, `${view.name}.png`), PNG.sync.write(diff));
    }
  }
} finally {
  await browser.close();
  server?.close();
}
process.exit(failed ? 1 : 0);
