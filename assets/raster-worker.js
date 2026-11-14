// Rasterizes the land mask off the main thread.
//
// Receives { land, lakes, width } (the raw .bin layers, see `decode_shapes` in
// src/geo.rs, and the map width in pixels). Replies with { tiles: [ArrayBuffer,
// ArrayBuffer] }: the left and right square halves of the width x width/2 map,
// one byte per pixel (the red channel), row 0 at the north. On any failure it
// replies { error } and the page falls back to drawing on the main thread.
//
// The drawing calls mirror `Layers::draw_tile` in src/geo.rs exactly (same
// transform, fill rules and colours) so both paths give the same pixels. Keep
// the two in sync. Classic worker (no modules): works in every browser that
// has OffscreenCanvas.
"use strict";

const COORD_SCALE = 10000;

// LEB128 varint as a Number (exact for values below 2^53).
function varint(data, pos) {
  let value = 0;
  let scale = 1;
  for (let i = 0; i < 10; i++) {
    if (pos.i >= data.length) throw new Error("unexpected end of data");
    const byte = data[pos.i++];
    value += (byte & 0x7f) * scale;
    if ((byte & 0x80) === 0) return value;
    scale *= 128;
  }
  throw new Error("varint too long");
}

function delta(data, pos) {
  const z = varint(data, pos);
  return z % 2 === 1 ? -(z + 1) / 2 : z / 2;
}

// Build a Path2D in lon/lat degrees from a "MAPD" v1 layer.
function toPath(data) {
  if (data.length < 5 || String.fromCharCode(...data.subarray(0, 4)) !== "MAPD" || data[4] !== 1) {
    throw new Error("not a version 1 map data file");
  }
  const pos = { i: 5 };
  const count = varint(data, pos);
  const path = new Path2D();
  let x = 0;
  let y = 0;
  for (let s = 0; s < count; s++) {
    const header = varint(data, pos);
    const n = Math.floor(header / 2);
    for (let p = 0; p < n; p++) {
      x += delta(data, pos);
      y += delta(data, pos);
      if (p === 0) path.moveTo(x / COORD_SCALE, y / COORD_SCALE);
      else path.lineTo(x / COORD_SCALE, y / COORD_SCALE);
    }
    if (header % 2 === 1) path.closePath();
  }
  return path;
}

function rasterize(landBytes, lakeBytes, width) {
  const h = width / 2;
  const land = toPath(new Uint8Array(landBytes));
  const lakes = toPath(new Uint8Array(lakeBytes));
  const canvas = new OffscreenCanvas(h, h);
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("2d OffscreenCanvas unavailable");
  const sx = width / 360;
  const sy = h / 180;
  const tiles = [];
  for (const xOffset of [0, h]) {
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.globalCompositeOperation = "source-over";
    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, h, h);
    // lon/lat degrees -> pixels: x = (lon + 180) * sx - offset, y = (90 - lat) * sy
    ctx.setTransform(sx, 0, 0, -sy, 180 * sx - xOffset, 90 * sy);
    ctx.fillStyle = "#f00";
    ctx.fill(land, "evenodd");
    ctx.fillStyle = "#000";
    ctx.fill(lakes, "nonzero");
    const rgba = ctx.getImageData(0, 0, h, h).data;
    const out = new Uint8Array(h * h);
    for (let i = 0, j = 0; i < out.length; i++, j += 4) out[i] = rgba[j];
    tiles.push(out.buffer);
  }
  return tiles;
}

self.onmessage = (e) => {
  try {
    const { land, lakes, width } = e.data;
    const tiles = rasterize(land, lakes, width);
    self.postMessage({ tiles }, tiles);
  } catch (err) {
    self.postMessage({ error: String(err && err.message ? err.message : err) });
  }
};
