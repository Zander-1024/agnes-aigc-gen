use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use reqwest::blocking::Client;

#[derive(Debug, Clone)]
pub enum ImageInput {
    Url(String),
    /// Raw base64 payload without the `data:` prefix.
    Base64(String),
    /// Already formatted: data:{mimetype};base64,{data}
    DataUri(String),
    LocalPath(String),
}

pub fn parse_image_inputs(values: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for value in values {
        for part in value.split(',') {
            let part = part.trim();
            if !part.is_empty() {
                out.push(part.to_string());
            }
        }
    }
    Ok(out)
}

pub fn classify_input(raw: &str) -> ImageInput {
    if raw.starts_with("http://") || raw.starts_with("https://") {
        ImageInput::Url(raw.to_string())
    } else if raw.starts_with("data:") {
        ImageInput::DataUri(raw.to_string())
    } else if let Some(rest) = raw.strip_prefix("base64:") {
        if rest.starts_with("data:") {
            ImageInput::DataUri(rest.to_string())
        } else {
            ImageInput::Base64(rest.to_string())
        }
    } else if looks_like_base64(raw) {
        ImageInput::Base64(raw.to_string())
    } else {
        ImageInput::LocalPath(raw.to_string())
    }
}

fn looks_like_base64(s: &str) -> bool {
    s.len() > 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
}

pub fn load_image_bytes(input: &ImageInput, client: &Client) -> Result<Vec<u8>> {
    match input {
        ImageInput::Url(url) => {
            let resp = client.get(url).send().context("fetch image url")?;
            let bytes = resp.bytes().context("read image url body")?;
            Ok(bytes.to_vec())
        }
        ImageInput::Base64(b64) => B64.decode(b64).context("decode base64 image"),
        ImageInput::DataUri(uri) => decode_data_uri_bytes(uri),
        ImageInput::LocalPath(path) => std::fs::read(path).with_context(|| format!("read local image {path}")),
    }
}

pub fn resolve_for_api(input: &ImageInput, _client: &Client) -> Result<String> {
    match input {
        ImageInput::Url(url) => Ok(url.clone()),
        ImageInput::DataUri(uri) => {
            if uri.starts_with("data:image/jpeg;base64,") {
                return Ok(uri.clone());
            }
            let bytes = decode_data_uri_bytes(uri)?;
            crate::media::local_bytes_to_data_uri(&bytes)
        }
        ImageInput::Base64(b64) => {
            let bytes = B64.decode(b64).context("decode base64 image")?;
            crate::media::local_bytes_to_data_uri(&bytes)
        }
        ImageInput::LocalPath(path) => {
            let bytes = std::fs::read(path).with_context(|| format!("read {path}"))?;
            crate::media::local_bytes_to_data_uri(&bytes)
        }
    }
}

fn decode_data_uri_bytes(uri: &str) -> Result<Vec<u8>> {
    let payload = uri.split_once(',').map(|(_, data)| data).context("parse data URI")?;
    B64.decode(payload).context("decode data URI base64")
}

pub fn image_dimensions_from_bytes(bytes: &[u8]) -> Result<(u32, u32)> {
    let img = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .context("guess image format")?
        .decode()
        .context("decode image")?;
    Ok((img.width(), img.height()))
}

pub fn ensure_same_ratio(all_dims: &[(u32, u32)]) -> Result<()> {
    if all_dims.len() <= 1 {
        return Ok(());
    }
    let first = crate::ratio::AspectRatio::from_dimensions(all_dims[0].0, all_dims[0].1);
    for &(w, h) in &all_dims[1..] {
        let r = crate::ratio::AspectRatio::from_dimensions(w, h);
        if r != first {
            bail!(
                "all input images must share the same aspect ratio; \
                 first={}:{} found {}:{}",
                first.w,
                first.h,
                r.w,
                r.h
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageBuffer, ImageFormat};

    #[test]
    fn local_png_converts_to_jpeg_data_uri() {
        let mut buf = Vec::new();
        ImageBuffer::from_fn(2, 2, |x, y| {
            if (x + y) % 2 == 0 {
                image::Rgba([255u8, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        })
        .write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::Png)
        .unwrap();

        let uri = crate::media::local_bytes_to_data_uri(&buf).unwrap();
        assert!(uri.starts_with("data:image/jpeg;base64,"));
    }

    #[test]
    fn local_jpeg_data_uri_unchanged() {
        let img = DynamicImage::new_rgb8(2, 2);
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::Jpeg)
            .unwrap();

        let uri = crate::media::local_bytes_to_data_uri(&buf).unwrap();
        assert!(uri.starts_with("data:image/jpeg;base64,"));
        let payload = uri.strip_prefix("data:image/jpeg;base64,").unwrap();
        assert_eq!(B64.decode(payload).unwrap(), buf);
    }

    #[test]
    fn data_uri_png_converts_to_jpeg() {
        let mut png = Vec::new();
        ImageBuffer::from_fn(1, 1, |_, _| image::Rgba([0u8, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let encoded = B64.encode(&png);
        let input = ImageInput::DataUri(format!("data:image/png;base64,{encoded}"));
        let out = resolve_for_api(&input, &Client::new()).unwrap();
        assert!(out.starts_with("data:image/jpeg;base64,"));
    }

    #[test]
    fn data_uri_jpeg_passthrough() {
        let input = ImageInput::DataUri("data:image/jpeg;base64,abcd".into());
        assert_eq!(
            resolve_for_api(&input, &Client::new()).unwrap(),
            "data:image/jpeg;base64,abcd"
        );
    }
}
