use anyhow::{Context, Result, bail};
use serde_json::json;

use crate::api::ApiClient;
use crate::api::types::{ExtraBodyImage, ImageGenerationRequest, ImageGenerationResponse};
use crate::logging;
use crate::media::{classify_input, parse_image_inputs, resolve_for_api};
use crate::output::{GenerationResult, OutputFormat, download_with_retry, infer_ext_from_url, write_base64_file};
use crate::ratio::{AspectRatio, image_dimensions};

pub struct ImageRequest {
    pub prompt: String,
    pub ratio: AspectRatio,
    pub inputs: Vec<String>,
    /// Fixed seed 0–999; when None, a random seed in that range is used.
    pub seed: Option<u32>,
    pub output_dir: Option<String>,
    pub save_local: bool,
    pub max_retries: Option<u32>,
    pub output_format: OutputFormat,
    pub quiet: bool,
}

pub fn generate_image(api: &ApiClient, req: ImageRequest) -> Result<GenerationResult> {
    let dims = image_dimensions(&req.ratio)?;
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

    log::debug!(
        "image generate size={} seed={seed} inputs={} save_local={}",
        dims.size_string(),
        parsed.len(),
        req.save_local
    );
    for (idx, raw) in parsed.iter().enumerate() {
        log::debug!("  input[{idx}]: {raw}");
    }

    let input_record = json!({
        "prompt": req.prompt,
        "ratio": req.ratio.label(),
        "size": dims.size_string(),
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
        size: dims.size_string(),
        extra_body: Some(extra),
    };

    let resp = api.post_json("images/generations", &body)?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        logging::log_response(status.as_u16(), &text);
        bail!("image generation failed ({status}): {text}");
    }
    let parsed: ImageGenerationResponse = resp.json().context("parse image response")?;

    let item = parsed.data.first().context("empty image response")?;
    let max_retries = req.max_retries.unwrap_or(api.config.max_retries);

    let remote_url = if let Some(url) = &item.url {
        url.clone()
    } else if let Some(b64) = &item.b64_json {
        if req.save_local {
            let output_dir = api.config.resolved_output_dir()?;
            let out_dir = req
                .output_dir
                .as_ref()
                .map(|d| crate::config::expand_tilde(d))
                .transpose()?
                .unwrap_or(output_dir);
            let path = write_base64_file(&out_dir, b64, "png")?;
            let result = GenerationResult {
                kind: "image".into(),
                ratio: req.ratio.label(),
                size: dims.size_string(),
                uri: path.display().to_string(),
                asset_uri: None,
                generation_id: None,
            };
            let output_record = serde_json::to_value(&result)?;
            let gen_id = api
                .db
                .insert_generation("image", Some(&req.prompt), &input_record, &output_record, None)?;
            let mut result = result;
            result.generation_id = Some(gen_id);
            if !req.quiet {
                result.print(req.output_format)?;
            }
            return Ok(result);
        }
        bail!("image response returned base64 only; use --save to write locally");
    } else {
        bail!("image response missing url and b64_json");
    };

    let uri = if req.save_local {
        log::debug!("downloading image to local output dir");
        let output_dir = api.config.resolved_output_dir()?;
        let out_dir = req
            .output_dir
            .as_ref()
            .map(|d| crate::config::expand_tilde(d))
            .transpose()?
            .unwrap_or(output_dir);
        let ext = infer_ext_from_url(&remote_url, "png");
        let path = download_with_retry(&api.client, &remote_url, &out_dir, &ext, max_retries)?;
        path.display().to_string()
    } else {
        remote_url.clone()
    };

    finish_image_result(api, req, dims.size_string(), uri, remote_url, input_record)
}

fn finish_image_result(
    api: &ApiClient,
    req: ImageRequest,
    size: String,
    uri: String,
    remote_url: String,
    input_record: serde_json::Value,
) -> Result<GenerationResult> {
    let asset = api
        .db
        .insert_asset("image", &remote_url, Some(&req.ratio.label()), Some(&size))?;

    let result = GenerationResult {
        kind: "image".into(),
        ratio: req.ratio.label(),
        size,
        uri,
        asset_uri: Some(asset.asset_uri.clone()),
        generation_id: None,
    };

    let output_record = serde_json::to_value(&result).context("serialize output")?;
    let gen_id = api.db.insert_generation(
        "image",
        Some(&req.prompt),
        &input_record,
        &output_record,
        Some(&asset.id),
    )?;

    let mut result = result;
    result.generation_id = Some(gen_id);

    log::debug!("recorded asset={} generation_id={gen_id}", asset.asset_uri);

    if !req.quiet {
        result.print(req.output_format)?;
    }
    Ok(result)
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
