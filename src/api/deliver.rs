use anyhow::{Context, Result};
use serde_json::Value;

use crate::api::ApiClient;
use crate::output::{GenerationResult, OutputFormat};

/// Output and persistence options shared by image and video finalize paths.
#[derive(Debug, Clone)]
pub struct DeliverOptions {
    pub output_format: OutputFormat,
    pub quiet: bool,
}

/// Asset metadata + URIs for a completed generation.
#[derive(Debug, Clone)]
pub struct DeliverPayload<'a> {
    pub kind: &'a str,
    pub prompt: Option<&'a str>,
    pub ratio: &'a str,
    pub size: &'a str,
    pub remote_url: &'a str,
    pub uri: String,
    pub input_record: Option<&'a Value>,
}

/// Record asset (+ optional generation) and optionally print the result.
pub fn deliver_generation(
    api: &ApiClient,
    payload: DeliverPayload<'_>,
    options: &DeliverOptions,
) -> Result<GenerationResult> {
    let asset = api.db.insert_asset(
        payload.kind,
        payload.remote_url,
        Some(payload.ratio),
        Some(payload.size),
    )?;

    let mut result = GenerationResult {
        kind: payload.kind.to_string(),
        ratio: payload.ratio.to_string(),
        size: payload.size.to_string(),
        uri: payload.uri,
        asset_uri: Some(asset.asset_uri.clone()),
        generation_id: None,
    };

    if let Some(input) = payload.input_record {
        let output_record = serde_json::to_value(&result).context("serialize output")?;
        let gen_id = api
            .db
            .insert_generation(payload.kind, payload.prompt, input, &output_record, Some(&asset.id))?;
        result.generation_id = Some(gen_id);
    }

    log::debug!(
        "recorded asset={} generation_id={:?}",
        asset.asset_uri,
        result.generation_id
    );

    if !options.quiet {
        result.print(options.output_format)?;
    }
    Ok(result)
}
