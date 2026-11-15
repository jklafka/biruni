// Compare two PNGs the way the visual test does (pixelmatch at PIXEL_THRESHOLD),
// but with no allowed fraction: any differing pixel counts as a change.
// Exit 0 if the dimensions match and 0 pixels differ, 1 otherwise (printing the
// differing-pixel count, or a size mismatch, on stdout), 2 on bad usage or an
// unreadable file.
//   node same-pixels.mjs a.png b.png
import { readFileSync } from "node:fs";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";
import { PIXEL_THRESHOLD } from "./thresholds.mjs";

const [a, b] = process.argv.slice(2);
if (!a || !b) {
  console.error("usage: same-pixels.mjs a.png b.png");
  process.exit(2);
}
try {
  const x = PNG.sync.read(readFileSync(a));
  const y = PNG.sync.read(readFileSync(b));
  if (x.width !== y.width || x.height !== y.height) {
    console.log(`size ${y.width}x${y.height} -> ${x.width}x${x.height}`);
    process.exit(1);
  }
  const n = pixelmatch(x.data, y.data, null, x.width, x.height, {
    threshold: PIXEL_THRESHOLD,
  });
  if (n > 0) console.log(`${n} px differ`);
  process.exit(n === 0 ? 0 : 1);
} catch (e) {
  console.error(`same-pixels: ${e.message}`);
  process.exit(2);
}
