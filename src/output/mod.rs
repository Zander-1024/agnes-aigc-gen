use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use chrono::Local;
use reqwest::blocking::Client;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const MAX_IMAGE_BATCH_COUNT: u32 = 4;

#[derive(Debug, Clone, Serialize)]
pub struct GenerationResult {
    #[serde(rename = "type")]
    pub kind: String,
    pub ratio: String,
    pub size: String,
    /// Remote URL, or local path when `--save` is used.
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<i64>,
}

/// One entry in a concurrent image batch (`-n` > 1). Same shape for success and failure;
/// unset fields are omitted from JSON.
#[derive(Debug, Clone, Serialize)]
pub struct ImageBatchItem {
    pub success: bool,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ImageBatchItem {
    pub fn from_result(result: GenerationResult) -> Self {
        Self {
            success: true,
            kind: Some(result.kind),
            ratio: Some(result.ratio),
            size: Some(result.size),
            uri: Some(result.uri),
            asset_uri: result.asset_uri,
            generation_id: result.generation_id,
            message: None,
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            kind: None,
            ratio: None,
            size: None,
            uri: None,
            asset_uri: None,
            generation_id: None,
            message: Some(message.into()),
        }
    }

    pub fn is_success(&self) -> bool {
        self.success
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;

    #[test]
    fn batch_item_success_includes_flag() {
        let item = ImageBatchItem::from_result(GenerationResult {
            kind: "image".into(),
            ratio: "1:1".into(),
            size: "1024x1024".into(),
            uri: "https://example.com/a.png".into(),
            asset_uri: Some("asset://abc".into()),
            generation_id: Some(1),
        });
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["type"], "image");
        assert!(json.get("message").is_none());
    }

    #[test]
    fn batch_item_failure_omits_empty_fields() {
        let item = ImageBatchItem::failure("image generation failed (429): rate limited");
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["success"], false);
        assert_eq!(json["message"], "image generation failed (429): rate limited");
        assert!(json.get("type").is_none());
        assert!(json.get("uri").is_none());
    }
}

impl GenerationResult {
    pub fn print(&self, format: OutputFormat) -> Result<()> {
        print_results(std::slice::from_ref(self), format)
    }
}

pub fn print_results(results: &[GenerationResult], format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            if results.len() == 1 {
                println!("{}", serde_json::to_string_pretty(&results[0])?);
            } else {
                println!("{}", serde_json::to_string_pretty(results)?);
            }
        }
        OutputFormat::Plain => {
            for result in results {
                println!("{}", result.uri);
            }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct BatchResultsResponse<'a> {
    results: &'a [ImageBatchItem],
}

pub fn print_batch_results(items: &[ImageBatchItem], format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            let output = BatchResultsResponse { results: items };
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        OutputFormat::Plain => {
            for item in items {
                if item.success {
                    if let Some(uri) = &item.uri {
                        println!("{uri}");
                    }
                } else if let Some(message) = &item.message {
                    eprintln!("{message}");
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    Json,
    Plain,
}

pub fn download_with_retry(
    client: &Client,
    url: &str,
    output_dir: &Path,
    ext: &str,
    max_retries: u32,
) -> Result<PathBuf> {
    fs::create_dir_all(output_dir)?;
    let filename = unique_filename(url, ext);
    let path = output_dir.join(filename);
    retry(max_retries, || {
        let bytes = client.get(url).send()?.bytes()?;
        fs::write(&path, &bytes)?;
        Ok(path.clone())
    })
}

pub fn write_base64_file(output_dir: &Path, b64: &str, ext: &str) -> Result<PathBuf> {
    fs::create_dir_all(output_dir)?;
    let bytes = B64.decode(b64).context("decode base64 output")?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let hash = format!("{:x}", hasher.finalize());
    let ts = Local::now().format("%Y%m%d-%H%M%S");
    let path = output_dir.join(format!("{ts}-{hash}.{ext}"));
    fs::write(&path, bytes)?;
    Ok(path)
}

fn unique_filename(url: &str, ext: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let ts = Local::now().format("%Y%m%d-%H%M%S");
    format!("{ts}-{hash}.{ext}")
}

pub fn retry<T, F>(max_retries: u32, mut f: F) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    let mut attempt = 0u32;
    loop {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => {
                attempt += 1;
                if attempt > max_retries {
                    return Err(e);
                }
                let delay = Duration::from_secs(1u64 << (attempt - 1).min(4));
                log::debug!("retry {attempt}/{max_retries} after error: {e:#}");
                thread::sleep(delay);
            }
        }
    }
}

pub fn infer_ext_from_url(url: &str, default: &str) -> String {
    url.rsplit('.')
        .next()
        .and_then(|s| s.split('?').next())
        .filter(|s| s.len() <= 5)
        .unwrap_or(default)
        .to_string()
}
