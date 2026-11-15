// Comparison limits shared by run.mjs and same-pixels.mjs.
// Per-pixel colour threshold (0..1) and the share of pixels allowed to differ.
// Loose enough for rasterizer/driver differences, tight enough to catch a
// flipped land mask, a black globe or a shifted camera.
export const PIXEL_THRESHOLD = 0.15;
export const MAX_DIFF_FRACTION = 0.005;
