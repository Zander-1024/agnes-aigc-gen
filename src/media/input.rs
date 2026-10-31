use anyhow::{Context, Result};
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
        let mut parts = value.split(',').map(str::trim);
        while let Some(part) = parts.next() {
            if part.is_empty() {
                continue;
            }
            if part.starts_with("data:") || part.starts_with("base64:data:") {
                // A Base64 Data URI contains one header separator; its payload has no commas.
                let payload = parts
                    .next()
                    .context("data URI requires a comma followed by a Base64 payload")?;
                anyhow::ensure!(!payload.is_empty(), "data URI requires a non-empty Base64 payload");
                out.push(format!("{part},{payload}"));
            } else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageBuffer, ImageFormat};

    #[test]
    fn data_uri_survives_input_parsing_and_reaches_image_conversion() {
        let mut png = Vec::new();
        DynamicImage::new_rgb8(2, 3)
            .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let data_uri = format!("data:image/png;base64,{}", B64.encode(png));
        for raw in [data_uri.clone(), format!("base64:{data_uri}")] {
            let inputs = parse_image_inputs(&[raw]).unwrap();
            assert_eq!(inputs.len(), 1);
            let output = resolve_for_api(&classify_input(&inputs[0]), &Client::new()).unwrap();
            let bytes = decode_data_uri_bytes(&output).unwrap();
            let image = image::load_from_memory(&bytes).unwrap();
            assert_eq!((image.width(), image.height()), (2, 3));
        }
    }

    #[test]
    fn mixed_comma_lists_and_repeated_inputs_preserve_data_uris() {
        let inputs = parse_image_inputs(&[
            " https://example.com/a.png, data:image/jpeg;base64,YWJj,asset://example ".into(),
            "base64:data:image/png;base64,ZGVm, ./photo.png".into(),
        ])
        .unwrap();
        assert_eq!(
            inputs,
            [
                "https://example.com/a.png",
                "data:image/jpeg;base64,YWJj",
                "asset://example",
                "base64:data:image/png;base64,ZGVm",
                "./photo.png"
            ]
        );
    }

    #[test]
    fn malformed_data_uris_fail_instead_of_becoming_missing_references() {
        for value in [
            "data:image/jpeg;base64",
            "data:image/jpeg;base64,",
            "base64:data:image/png;base64, ,https://example.com/image.png",
        ] {
            assert!(parse_image_inputs(&[value.into()]).is_err(), "{value}");
        }
        assert!(parse_image_inputs(&[" , ".into()]).unwrap().is_empty());
    }

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
