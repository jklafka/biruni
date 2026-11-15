// Exit 0 if two PNGs have identical dimensions and decoded RGBA pixels,
// 1 if they differ, 2 on bad usage or an unreadable file.
//   node same-pixels.mjs a.png b.png
import { readFileSync } from "node:fs";
import { PNG } from "pngjs";

const [a, b] = process.argv.slice(2);
if (!a || !b) {
  console.error("usage: same-pixels.mjs a.png b.png");
  process.exit(2);
}
try {
  const x = PNG.sync.read(readFileSync(a));
  const y = PNG.sync.read(readFileSync(b));
  const same =
    x.width === y.width && x.height === y.height && Buffer.compare(x.data, y.data) === 0;
  process.exit(same ? 0 : 1);
} catch (e) {
  console.error(`same-pixels: ${e.message}`);
  process.exit(2);
}
