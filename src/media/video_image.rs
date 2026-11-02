use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use reqwest::Url;

/// Resolve an image after any `asset://` reference has been expanded.
pub fn resolve_video_image_input(raw: &str) -> Result<String> {
    let raw = raw.trim();
    ensure!(!raw.is_empty(), "video image input must not be empty");
    let path = crate::config::expand_tilde(raw)?;
    if path.is_file() {
        return validated_jpeg_data_uri(&std::fs::read(path).context("read local video image")?, None);
    }
    if raw.starts_with("http://") || raw.starts_with("https://") {
        let url = Url::parse(raw).context("invalid video image URL")?;
        ensure!(
            url.scheme() == "https" && url.host_str().is_some(),
            "video image URL must use HTTPS and include a host"
        );
        return Ok(raw.to_string());
    }

    let inline = raw.strip_prefix("base64:").unwrap_or(raw);
    if inline.starts_with("data:") {
        let (header, payload) = inline
            .split_once(',')
            .context("video image Data URI requires a Base64 payload")?;
        let declared_mime = match header.to_ascii_lowercase().as_str() {
            "data:image/jpeg;base64" | "data:image/jpg;base64" => "image/jpeg",
            "data:image/png;base64" => "image/png",
            "data:image/webp;base64" => "image/webp",
            _ => anyhow::bail!("video image Data URI must use data:image/jpeg, png, or webp;base64,..."),
        };
        return validated_jpeg_data_uri(&decode_base64(payload)?, Some(declared_mime));
    }
    if raw.starts_with("base64:") {
        return validated_jpeg_data_uri(&decode_base64(inline)?, None);
    }
    // Existing files win above; recognize short and line-wrapped payloads too.
    if let Ok(bytes) = decode_base64(raw)
        && let Ok(mime) = crate::media::jpeg::mime_from_bytes(&bytes)
        && matches!(mime, "image/jpeg" | "image/png" | "image/webp")
    {
        return validated_jpeg_data_uri(&bytes, None);
    }
    validated_jpeg_data_uri(&std::fs::read(path).context("read local video image")?, None)
}

fn decode_base64(payload: &str) -> Result<Vec<u8>> {
    let compact: Vec<u8> = payload.bytes().filter(|byte| !byte.is_ascii_whitespace()).collect();
    ensure!(!compact.is_empty(), "video image Base64 payload must not be empty");
    B64.decode(compact).context("invalid video image Base64 payload")
}

fn validated_jpeg_data_uri(bytes: &[u8], declared_mime: Option<&str>) -> Result<String> {
    let mime = crate::media::jpeg::mime_from_bytes(bytes)?;
    ensure!(
        matches!(mime, "image/jpeg" | "image/png" | "image/webp"),
        "video image must be PNG, JPEG, or WebP"
    );
    if let Some(declared_mime) = declared_mime {
        ensure!(
            mime == declared_mime,
            "video image Data URI media type does not match its image bytes"
        );
    }
    // Validate JPEG bytes as well; format sniffing alone can accept broken files.
    image::load_from_memory(bytes).context("decode video image")?;
    crate::media::local_bytes_to_data_uri(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat};
    use std::path::PathBuf;

    fn image_bytes(format: ImageFormat) -> Vec<u8> {
        let mut bytes = Vec::new();
        DynamicImage::new_rgb8(2, 3)
            .write_to(&mut std::io::Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    fn jpeg_bytes(uri: &str) -> Vec<u8> {
        let payload = uri.strip_prefix("data:image/jpeg;base64,").unwrap();
        B64.decode(payload).unwrap()
    }

    #[test]
    fn png_jpeg_and_webp_inline_inputs_become_valid_jpeg_data_uris() {
        for (format, mime) in
            [(ImageFormat::Png, "image/png"), (ImageFormat::Jpeg, "image/jpeg"), (ImageFormat::WebP, "image/webp")]
        {
            let bytes = image_bytes(format);
            let payload = B64.encode(&bytes);
            for raw in [
                payload.clone(),
                format!("base64:{payload}"),
                format!("data:{mime};base64,{payload}"),
                format!("base64:data:{mime};base64,{payload}"),
            ] {
                let output = resolve_video_image_input(&raw).unwrap();
                let output_bytes = jpeg_bytes(&output);
                assert_eq!(image::guess_format(&output_bytes).unwrap(), ImageFormat::Jpeg);
                let image = image::load_from_memory(&output_bytes).unwrap();
                assert_eq!((image.width(), image.height()), (2, 3));
                if format == ImageFormat::Jpeg {
                    assert_eq!(output_bytes, bytes, "JPEG inputs must not be re-encoded");
                }
            }
        }
    }

    #[test]
    fn short_webp_and_line_wrapped_base64_are_not_mistaken_for_paths() {
        let mut bytes = Vec::new();
        DynamicImage::new_rgb8(1, 1)
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::WebP)
            .unwrap();
        let payload = B64.encode(bytes);
        assert!(payload.len() <= 64, "fixture should exercise the short-Base64 case");
        let wrapped = payload
            .as_bytes()
            .chunks(8)
            .map(|part| std::str::from_utf8(part).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let expected = resolve_video_image_input(&payload).unwrap();
        assert_eq!(resolve_video_image_input(&wrapped).unwrap(), expected);
        assert_eq!(
            resolve_video_image_input(&format!("base64:{wrapped}")).unwrap(),
            expected
        );
        assert_eq!(
            resolve_video_image_input(&format!("data:image/webp;base64,{wrapped}")).unwrap(),
            expected
        );
    }

    struct LocalImage(PathBuf);

    impl LocalImage {
        fn new(bytes: &[u8]) -> Self {
            let path = std::env::temp_dir().join(format!("agnes-video-inline-{}.image", rand::random::<u64>()));
            std::fs::write(&path, bytes).unwrap();
            Self(path)
        }
    }

    impl Drop for LocalImage {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn local_files_use_actual_format_and_support_tilde_expansion() {
        let bytes = image_bytes(ImageFormat::Png);
        let file = LocalImage::new(&bytes);
        let output = resolve_video_image_input(file.0.to_str().unwrap()).unwrap();
        assert_eq!(image::guess_format(&jpeg_bytes(&output)).unwrap(), ImageFormat::Jpeg);
        assert_eq!(
            std::fs::read(&file.0).unwrap(),
            bytes,
            "original file must remain unchanged"
        );

        let home = dirs::home_dir().unwrap();
        let levels = home
            .components()
            .filter(|part| matches!(part, std::path::Component::Normal(_)))
            .count();
        let root = home.ancestors().last().unwrap();
        if let Ok(relative) = file.0.strip_prefix(root) {
            let tilde_path = format!("~{}/{}", "/..".repeat(levels), relative.display());
            assert_eq!(resolve_video_image_input(&tilde_path).unwrap(), output);
        }
    }

    #[test]
    fn urls_are_forwarded_without_download_or_conversion() {
        let url = "https://example.com/input.png?signature=value&crop=1,2";
        assert_eq!(resolve_video_image_input(url).unwrap(), url);
        for invalid in ["http://example.com/input.png", "https:///", "file:///tmp/image.png", "asset://unresolved"] {
            assert!(resolve_video_image_input(invalid).is_err());
        }
    }

    #[test]
    fn invalid_inline_payloads_and_non_image_headers_are_rejected() {
        let payload = B64.encode(image_bytes(ImageFormat::Png));
        for raw in [
            String::new(),
            "data:image/jpeg;base64,AAAA".into(),
            "data:image/png;base64,".into(),
            "data:image/png;base64,%%%".into(),
            "base64:not-an-image".into(),
            format!("data:text/plain;base64,{payload}"),
            format!("data:image/png,{payload}"),
            format!("data:image/jpeg;base64,{payload}"),
        ] {
            assert!(resolve_video_image_input(&raw).is_err(), "invalid input was accepted");
        }
    }
}
