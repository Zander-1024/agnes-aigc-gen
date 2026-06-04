use anyhow::{Context, Result};
use image::{DynamicImage, ImageFormat};

/// Return JPEG bytes unchanged; encode other formats as JPEG (same dimensions).
pub fn ensure_jpeg_bytes(bytes: &[u8]) -> Result<Vec<u8>> {
    if is_jpeg_bytes(bytes)? {
        return Ok(bytes.to_vec());
    }
    let img = decode_image(bytes)?;
    encode_as_jpeg(&img)
}

pub fn is_jpeg_bytes(bytes: &[u8]) -> Result<bool> {
    Ok(mime_from_bytes(bytes)? == "image/jpeg")
}

pub fn encode_as_jpeg(img: &DynamicImage) -> Result<Vec<u8>> {
    let rgb = img.to_rgb8();
    let mut buf = Vec::new();
    DynamicImage::ImageRgb8(rgb)
        .write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::Jpeg)
        .context("encode jpeg")?;
    Ok(buf)
}

fn decode_image(bytes: &[u8]) -> Result<DynamicImage> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .context("guess image format")?
        .decode()
        .context("decode image")
}

pub fn mime_from_bytes(bytes: &[u8]) -> Result<&'static str> {
    let format = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .context("guess image format for mimetype")?
        .format();
    Ok(match format {
        Some(ImageFormat::Jpeg) => "image/jpeg",
        Some(ImageFormat::Png) => "image/png",
        Some(ImageFormat::WebP) => "image/webp",
        Some(ImageFormat::Gif) => "image/gif",
        Some(ImageFormat::Bmp) => "image/bmp",
        Some(ImageFormat::Tiff) => "image/tiff",
        _ => "application/octet-stream",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageBuffer, ImageFormat};

    #[test]
    fn png_converts_to_jpeg() {
        let mut png = Vec::new();
        ImageBuffer::from_fn(2, 2, |_, _| image::Rgba([1u8, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let jpeg = ensure_jpeg_bytes(&png).unwrap();
        assert!(is_jpeg_bytes(&jpeg).unwrap());
        assert_ne!(jpeg, png);
    }

    #[test]
    fn jpeg_passthrough() {
        let img = DynamicImage::new_rgb8(2, 2);
        let mut jpeg = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut jpeg), ImageFormat::Jpeg)
            .unwrap();
        let out = ensure_jpeg_bytes(&jpeg).unwrap();
        assert_eq!(out, jpeg);
    }
}
