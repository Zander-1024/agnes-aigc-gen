use std::path::Path;

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use pi_ai::{Content, Message};
use reqwest::Url;

use crate::media::input::ImageInput;
use crate::media::{classify_input, jpeg::mime_from_bytes};

pub const DEFAULT_IMAGE_PROMPT: &str = "Describe the attached image(s).";

/// Build a self-contained turn so retries and saved sessions retain inline images.
/// Each input is one image; commas in paths, URLs, and Data URIs remain intact.
pub fn build_user_message(prompt: &str, images: &[String]) -> Result<Message> {
    if images.is_empty() {
        return Ok(Message::user_text(prompt));
    }
    let prompt = if prompt.trim().is_empty() {
        DEFAULT_IMAGE_PROMPT
    } else {
        prompt
    };
    let mut content = vec![Content::text(prompt)];
    for (index, input) in images.iter().enumerate() {
        content.push(resolve_image(input).with_context(|| format!("chat image {}", index + 1))?);
    }
    Ok(Message::User { content, timestamp: pi_ai::now_ms() })
}

fn resolve_image(raw: &str) -> Result<Content> {
    let raw = raw.trim();
    anyhow::ensure!(!raw.is_empty(), "image input must not be empty");
    // Existing local files take precedence over the bare-Base64 heuristic.
    let path = crate::config::expand_tilde(raw)?;
    if Path::new(&path).is_file() {
        return inline_image(&std::fs::read(&path).context("read local chat image")?);
    }
    match classify_input(raw) {
        ImageInput::Url(url) => {
            let parsed = Url::parse(&url).context("invalid image URL")?;
            anyhow::ensure!(parsed.host_str().is_some(), "image URL requires a host");
            Ok(Content::Image { data: url, mime_type: "image/*".into() })
        }
        ImageInput::DataUri(uri) => {
            let (header, payload) = uri
                .split_once(',')
                .context("image Data URI requires a Base64 payload")?;
            anyhow::ensure!(
                header.starts_with("data:image/") && header.ends_with(";base64"),
                "image Data URI must use data:image/<type>;base64,..."
            );
            let bytes = decode_base64(payload)?;
            inline_image(&bytes)
        }
        ImageInput::Base64(payload) => {
            let bytes = decode_base64(&payload)?;
            inline_image(&bytes)
        }
        ImageInput::LocalPath(_) => {
            // Small WebP images and line-wrapped Base64 can miss the generic input heuristic.
            if let Ok(bytes) = decode_base64(raw)
                && let Ok(mime_type) = mime_from_bytes(&bytes)
                && matches!(mime_type, "image/jpeg" | "image/png" | "image/webp")
            {
                return inline_image(&bytes);
            }
            inline_image(&std::fs::read(path).context("read local chat image")?)
        }
    }
}

fn decode_base64(payload: &str) -> Result<Vec<u8>> {
    let compact: Vec<u8> = payload.bytes().filter(|byte| !byte.is_ascii_whitespace()).collect();
    B64.decode(compact).context("invalid Base64 image payload")
}

fn inline_image(bytes: &[u8]) -> Result<Content> {
    let mime_type = mime_from_bytes(bytes)?;
    anyhow::ensure!(
        matches!(mime_type, "image/jpeg" | "image/png" | "image/webp"),
        "chat image must be a PNG, JPEG, or WebP image"
    );
    image::load_from_memory(bytes).context("decode chat image")?;
    Ok(Content::Image { data: B64.encode(bytes), mime_type: mime_type.into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat};

    fn png_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        DynamicImage::new_rgb8(2, 3)
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn image_data(message: &Message) -> (&str, &str) {
        let Message::User { content, .. } = message else {
            panic!("expected user message")
        };
        let Content::Image { data, mime_type } = &content[1] else {
            panic!("expected image")
        };
        (data, mime_type)
    }

    #[test]
    fn image_urls_are_forwarded_without_fetching_or_splitting_commas() {
        let url = "https://example.com/photo.png?crop=1,2,3,4";
        let message = build_user_message("describe", &[url.into()]).unwrap();
        assert_eq!(image_data(&message), (url, "image/*"));
    }

    #[test]
    fn local_file_and_all_base64_forms_retain_image_bytes() {
        let bytes = png_bytes();
        let payload = B64.encode(&bytes);
        let wrapped = payload
            .as_bytes()
            .chunks(16)
            .map(|part| std::str::from_utf8(part).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let path = std::env::temp_dir().join(format!(
            "agnes-chat-image-{},{}.png",
            std::process::id(),
            pi_ai::now_ms()
        ));
        std::fs::write(&path, &bytes).unwrap();
        for raw in [
            path.to_string_lossy().to_string(),
            payload.clone(),
            wrapped.clone(),
            format!("base64:{payload}"),
            format!("base64:{wrapped}"),
            format!("data:image/png;base64,{payload}"),
            format!("data:image/png;base64,{wrapped}"),
            format!("base64:data:image/png;base64,{payload}"),
        ] {
            let message = build_user_message("describe", &[raw]).unwrap();
            let (data, mime_type) = image_data(&message);
            assert_eq!(mime_type, "image/png");
            assert_eq!(B64.decode(data).unwrap(), bytes);
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn short_bare_base64_webp_is_not_mistaken_for_a_local_path() {
        let mut bytes = Vec::new();
        DynamicImage::new_rgb8(1, 1)
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::WebP)
            .unwrap();
        let payload = B64.encode(&bytes);
        assert!(payload.len() <= 64);
        let message = build_user_message("describe", &[payload]).unwrap();
        let (data, mime_type) = image_data(&message);
        assert_eq!(mime_type, "image/webp");
        assert_eq!(B64.decode(data).unwrap(), bytes);
    }

    #[test]
    fn multiple_images_and_image_only_prompts_are_preserved() {
        let message = build_user_message(
            "",
            &["https://example.com/a.png".into(), "https://example.com/b.png".into()],
        )
        .unwrap();
        let Message::User { content, .. } = message else {
            panic!("expected user message")
        };
        assert_eq!(content.len(), 3);
        assert_eq!(content[0].as_text(), Some(DEFAULT_IMAGE_PROMPT));
        let plain = build_user_message("hello", &[]).unwrap();
        let Message::User { content, .. } = plain else {
            panic!("expected user message")
        };
        assert_eq!(content, vec![Content::text("hello")]);
    }

    #[test]
    fn invalid_images_fail_before_a_request_is_built() {
        for input in ["", "data:image/png;base64,", "base64:invalid!", "data:text/plain;base64,YWJj"] {
            assert!(build_user_message("describe", &[input.into()]).is_err());
        }
        let invalid_bytes = format!("base64:{}", B64.encode(b"not an image"));
        assert!(build_user_message("describe", &[invalid_bytes]).is_err());
    }
}
