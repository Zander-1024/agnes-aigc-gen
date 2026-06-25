use anyhow::{Context, Result, bail};
use serde_json::json;

use crate::api::ApiClient;
use crate::api::deliver::{DeliverOptions, DeliverPayload, deliver_generation};
use crate::api::types::{ExtraBodyImage, ImageGenerationRequest, ImageGenerationResponse};
use crate::media::{classify_input, parse_image_inputs, resolve_for_api};
use crate::output::{OutputFormat, download_with_retry, infer_ext_from_url, write_base64_file};
use crate::ratio::{AspectRatio, ImageResolutionTier, validate_image_ratio};

pub struct ImageRequest {
    pub prompt: String,
    pub ratio: AspectRatio,
    pub size_tier: ImageResolutionTier,
    pub inputs: Vec<String>,
    /// Fixed seed 0–999; when None, a random seed in that range is used.
    pub seed: Option<u32>,
    pub output_dir: Option<String>,
    pub save_local: bool,
    pub max_retries: Option<u32>,
    pub output_format: OutputFormat,
    pub quiet: bool,
}

pub fn generate_image(api: &ApiClient, req: ImageRequest) -> Result<crate::output::GenerationResult> {
    validate_image_ratio(&req.ratio)?;
    let size_tier = req.size_tier.as_str();
    let ratio_label = req.ratio.label();
    let parsed = parse_image_inputs(&req.inputs)?;
    let mut resolved_inputs = Vec::new();
    let mut image_urls = Vec::new();
    for raw in &parsed {
        let resolved = api.db.resolve_reference(raw)?;
        resolved_inputs.push(resolved.clone());
        let classified = classify_input(&resolved);
        image_urls.push(resolve_for_api(&classified, &api.client)?);
    }

    let seed = resolve_image_seed(req.seed)?;
    let deliver_options = DeliverOptions { output_format: req.output_format, quiet: req.quiet };

    log::debug!(
        "image generate ratio={ratio_label} size={size_tier} seed={seed} inputs={} save_local={}",
        parsed.len(),
        req.save_local
    );
    for (idx, raw) in parsed.iter().enumerate() {
        log::debug!("  input[{idx}]: {raw}");
    }

    let input_record = json!({
        "prompt": req.prompt,
        "ratio": ratio_label,
        "size": size_tier,
        "seed": seed,
        "inputs": parsed,
        "resolved_inputs": resolved_inputs,
        "model": api.config.image_model,
    });

    let extra = ExtraBodyImage {
        image: if image_urls.is_empty() { None } else { Some(image_urls) },
        response_format: Some("url".into()),
        seed: Some(seed),
    };

    let body = ImageGenerationRequest {
        model: api.config.image_model.clone(),
        prompt: req.prompt.clone(),
        ratio: ratio_label.clone(),
        size: size_tier.to_string(),
        extra_body: Some(extra),
    };

    let resp = api.post_json("images/generations", &body)?;
    let resp = ApiClient::ensure_success(resp, "image generation")?;
    let parsed: ImageGenerationResponse = resp
        .json()
        .map_err(|e| anyhow::Error::new(crate::api::error::AgnesApiError::Parse(e.to_string())))?;

    let item = parsed.data.first().context("empty image response")?;
    let max_retries = req.max_retries.unwrap_or(api.config.max_retries);
    let size = size_tier.to_string();

    let remote_url = if let Some(url) = &item.url {
        url.clone()
    } else if let Some(b64) = &item.b64_json {
        if !req.save_local {
            bail!("image response returned base64 only; use --save to write locally");
        }
        let out_dir = resolve_output_dir(api, req.output_dir.as_deref())?;
        let path = write_base64_file(&out_dir, b64, "png")?;
        let uri = path.display().to_string();
        return deliver_generation(
            api,
            DeliverPayload {
                kind: "image",
                prompt: Some(&req.prompt),
                ratio: &ratio_label,
                size: &size,
                remote_url: &uri,
                uri: uri.clone(),
                input_record: Some(&input_record),
            },
            &deliver_options,
        );
    } else {
        bail!("image response missing url and b64_json");
    };

    let uri = if req.save_local {
        log::debug!("downloading image to local output dir");
        let out_dir = resolve_output_dir(api, req.output_dir.as_deref())?;
        let ext = infer_ext_from_url(&remote_url, "png");
        let path = download_with_retry(&api.client, &remote_url, &out_dir, &ext, max_retries)?;
        path.display().to_string()
    } else {
        remote_url.clone()
    };

    deliver_generation(
        api,
        DeliverPayload {
            kind: "image",
            prompt: Some(&req.prompt),
            ratio: &ratio_label,
            size: &size,
            remote_url: &remote_url,
            uri,
            input_record: Some(&input_record),
        },
        &deliver_options,
    )
}

fn resolve_output_dir(api: &ApiClient, override_dir: Option<&str>) -> Result<std::path::PathBuf> {
    let output_dir = api.config.resolved_output_dir()?;
    Ok(override_dir
        .map(crate::config::expand_tilde)
        .transpose()?
        .unwrap_or(output_dir))
}

/// Seed for image generation: explicit 0–999, or random perturbation in that range.
pub fn resolve_image_seed(explicit: Option<u32>) -> Result<u32> {
    if let Some(seed) = explicit {
        anyhow::ensure!(seed <= 999, "seed must be 0–999, got {seed}");
        return Ok(seed);
    }
    Ok(rand::random_range(0..=999))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_range() {
        assert!(resolve_image_seed(Some(999)).is_ok());
        assert!(resolve_image_seed(Some(1000)).is_err());
        let s = resolve_image_seed(None).unwrap();
        assert!(s <= 999);
    }
}
