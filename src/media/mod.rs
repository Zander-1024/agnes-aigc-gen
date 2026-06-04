pub mod input;
pub mod jpeg;

pub use input::{classify_input, parse_image_inputs, resolve_for_api};
pub use jpeg::ensure_jpeg_bytes;

use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

/// `data:image/jpeg;base64,{b64_data}` for image and video API upload payloads.
pub fn jpeg_bytes_to_data_uri(bytes: &[u8]) -> Result<String> {
    Ok(format!("data:image/jpeg;base64,{}", B64.encode(bytes)))
}

/// Local / inline image bytes → JPEG (if needed) → data URI.
pub fn local_bytes_to_data_uri(bytes: &[u8]) -> Result<String> {
    let jpeg = ensure_jpeg_bytes(bytes)?;
    jpeg_bytes_to_data_uri(&jpeg)
}
