//! Decoding of the satellite imagery file (the Imagery layer's texture).
//!
//! The file is a lossy WebP (see `scripts/make_blue_marble.py`). The browser
//! decodes it with `createImageBitmap`, which also resizes it, so the low
//! texture tier gets 2048x1024 without ever holding the 4096x2048 pixels in
//! the page. Only the encoded bytes (about 0.7 MB) are kept for restoring
//! the texture after a lost WebGL context; the bitmap is closed after upload.

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Blob, BlobPropertyBag, ImageBitmap, ImageBitmapOptions, ResizeQuality};

/// Height of an equirectangular image `width` wide.
pub fn height_for(width: u32) -> u32 {
    width / 2
}

/// Number of mip levels of a `width` x `height` texture, down to 1x1.
pub fn mip_levels(width: u32, height: u32) -> i32 {
    32 - width.max(height).max(1).leading_zeros() as i32
}

/// Decode `bytes` to a bitmap `width` pixels wide (and half as tall). A
/// browser that ignores the resize options returns the full-size image,
/// which the caller sizes its texture from.
pub async fn decode(bytes: &[u8], width: u32) -> Result<ImageBitmap, JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let props = BlobPropertyBag::new();
    props.set_type("image/webp");
    let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &props)?;
    let opts = ImageBitmapOptions::new();
    opts.set_resize_width(width);
    opts.set_resize_height(height_for(width));
    opts.set_resize_quality(ResizeQuality::High);
    let promise = window.create_image_bitmap_with_blob_and_image_bitmap_options(&blob, &opts)?;
    JsFuture::from(promise)
        .await
        .map_err(|e| {
            JsValue::from_str(&format!(
                "could not decode the satellite image: {}",
                e.as_string().unwrap_or_else(|| format!("{e:?}"))
            ))
        })?
        .dyn_into::<ImageBitmap>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_lengths() {
        assert_eq!(mip_levels(4096, 2048), 13);
        assert_eq!(mip_levels(2048, 1024), 12);
        assert_eq!(mip_levels(1, 1), 1);
        assert_eq!(mip_levels(0, 0), 1);
        assert_eq!(height_for(4096), 2048);
    }
}
