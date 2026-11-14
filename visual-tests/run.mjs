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
import { createServer } from "node:http";
import { readFile, mkdir, writeFile, rm } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const dist = join(here, "..", "dist");
const update = process.argv.includes("--update");

// Per-pixel colour threshold (0..1) and the share of pixels allowed to differ.
// Loose enough for rasterizer/driver differences, tight enough to catch a
// flipped land mask, a black globe or a shifted camera.
const PIXEL_THRESHOLD = 0.15;
const MAX_DIFF_FRACTION = 0.005;

const VIEWPORT = { width: 800, height: 600 };
const VIEWS = [
  { name: "globe", hash: "" },
  { name: "mid-zoom", hash: "#38.5,22.5,1500" },
  { name: "close-zoom", hash: "#47,8.5,400" },
  { name: "antimeridian", hash: "#-10,175,9000" },
  { name: "mono-mid-zoom", hash: "#38.5,22.5,1500&layer=mono" },
  // The sun is pinned with &t=<unix seconds> so these never depend on the
  // clock: 2024-06-21 18:00 UTC, sun over about 90 W.
  { name: "day-night", hash: "#30,0,9000&sun=on&t=1718992800" },
  { name: "mono-day-night", hash: "#30,0,9000&layer=mono&sun=on&t=1718992800" },
];

const MIME = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".geojson": "application/json",
  ".css": "text/css",
  ".png": "image/png",
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
    const page = await browser.newPage({ viewport: VIEWPORT, deviceScaleFactor: 1 });
    const problems = [];
    page.on("pageerror", (e) => problems.push(String(e)));
    page.on("console", (m) => m.type() === "error" && problems.push(m.text()));

    await page.goto(base + view.hash);
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
    await page.evaluate(
      () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))),
    );
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
