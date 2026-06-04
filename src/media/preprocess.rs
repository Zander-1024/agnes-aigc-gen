use anyhow::{Context, Result};
use image::DynamicImage;
use image::imageops::FilterType;
use reqwest::blocking::Client;

use crate::media::input::{ImageInput, ensure_same_ratio, image_dimensions_from_bytes, load_image_bytes};
use crate::media::{encode_as_jpeg, jpeg_bytes_to_data_uri};
use crate::ratio::{AspectRatio, Dimensions, video_dimensions};

#[derive(Debug, Clone)]
pub struct PreparedFrame {
    /// `data:image/jpeg;base64,...` — same format as image API inputs.
    pub payload: String,
    #[allow(dead_code)]
    pub ratio: AspectRatio,
    #[allow(dead_code)]
    pub dimensions: Dimensions,
}

pub fn prepare_video_frames(
    inputs: &[ImageInput],
    client: &Client,
) -> Result<(Vec<PreparedFrame>, AspectRatio, Dimensions)> {
    let mut dims = Vec::new();
    let mut decoded = Vec::new();
    for input in inputs {
        let bytes = load_image_bytes(input, client)?;
        dims.push(image_dimensions_from_bytes(&bytes)?);
        decoded.push(bytes);
    }
    ensure_same_ratio(&dims)?;
    let ratio = AspectRatio::from_dimensions(dims[0].0, dims[0].1);
    let target = video_dimensions(&ratio);
    let mut frames = Vec::new();
    for bytes in decoded {
        let img = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .context("guess frame format")?
            .decode()
            .context("decode frame")?;
        let jpeg = fit_cover_to_jpeg(&img, target.width, target.height)?;
        frames.push(PreparedFrame {
            payload: jpeg_bytes_to_data_uri(&jpeg)?,
            ratio: ratio.clone(),
            dimensions: target,
        });
    }
    Ok((frames, ratio, target))
}

fn fit_cover_to_jpeg(img: &DynamicImage, width: u32, height: u32) -> Result<Vec<u8>> {
    let (src_w, src_h) = (img.width() as f64, img.height() as f64);
    let scale = (width as f64 / src_w).max(height as f64 / src_h);
    let scaled_w = (src_w * scale).ceil() as u32;
    let scaled_h = (src_h * scale).ceil() as u32;
    let scaled = img.resize(scaled_w, scaled_h, FilterType::Lanczos3);
    let x = scaled_w.saturating_sub(width) / 2;
    let y = scaled_h.saturating_sub(height) / 2;
    let cropped = scaled.crop_imm(x, y, width, height);
    encode_as_jpeg(&cropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_crop_jpeg() {
        let img = DynamicImage::new_rgb8(100, 50);
        let out = fit_cover_to_jpeg(&img, 64, 64).unwrap();
        assert!(!out.is_empty());
        let decoded = image::load_from_memory(&out).unwrap();
        assert_eq!(decoded.width(), 64);
        assert_eq!(decoded.height(), 64);
    }

    #[test]
    fn video_frame_uses_jpeg_data_uri() {
        use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
        use image::{ImageBuffer, ImageFormat};

        let mut png = Vec::new();
        ImageBuffer::from_fn(400, 400, |_, _| image::Rgba([255u8, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let img = image::load_from_memory(&png).unwrap();
        let jpeg = fit_cover_to_jpeg(&img, 1024, 1024).unwrap();
        assert!(crate::media::jpeg::is_jpeg_bytes(&jpeg).unwrap());

        let payload = crate::media::jpeg_bytes_to_data_uri(&jpeg).unwrap();
        assert!(payload.starts_with("data:image/jpeg;base64,"));
        let b64 = payload.strip_prefix("data:image/jpeg;base64,").unwrap();
        let decoded = B64.decode(b64).unwrap();
        assert!(crate::media::jpeg::is_jpeg_bytes(&decoded).unwrap());
    }
}
