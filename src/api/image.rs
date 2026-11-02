use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::api::ApiClient;
use crate::api::deliver::{DeliverOptions, DeliverPayload, deliver_generation};
use crate::api::types::{ExtraBodyImage, ImageGenerationRequest, ImageGenerationResponse};
use crate::config::RequestKind;
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
    /// Fields explicitly supplied by the caller; omitted fields retain defaults.
    pub explicit: Value,
    pub batch_count: u32,
    pub output_dir: Option<String>,
    pub save_local: bool,
    pub max_retries: Option<u32>,
    pub output_format: OutputFormat,
    pub quiet: bool,
}

pub fn generate_image(api: &ApiClient, req: ImageRequest) -> Result<crate::output::GenerationResult> {
    api.last_request_id.set(None);
    let result = generate_image_inner(api, req);
    if let Some(id) = api.last_request_id.get() {
        api.db.finish_request_run(
            id,
            if result.is_ok() { "succeeded" } else { "failed" },
            None,
            result
                .as_ref()
                .err()
                .map(|error| crate::request_log::error_summary(&error.to_string()))
                .as_deref(),
        )?;
    }
    result
}

fn generate_image_inner(api: &ApiClient, req: ImageRequest) -> Result<crate::output::GenerationResult> {
    let prepared = prepare_image_body(api, &req)?;
    let body = prepared.body;
    let ratio_label = prepared.ratio;
    let size = prepared.size;
    let mut input_record = prepared.input_record;
    let deliver_options = DeliverOptions { output_format: req.output_format, quiet: req.quiet };

    let metadata = &input_record["request_config"];
    let layers: Vec<String> = serde_json::from_value(metadata["layers"].clone())?;
    let resp = api.post_json_with_config(
        "images/generations",
        &body,
        metadata["hash"].as_str().context("missing image configuration hash")?,
        &layers,
    )?;
    input_record["request_id"] = json!(api.last_request_id.get());
    let resp = ApiClient::ensure_success(resp, "image generation")?;
    let parsed: ImageGenerationResponse = resp
        .json()
        .map_err(|e| anyhow::Error::new(crate::api::error::AgnesApiError::Parse(e.to_string())))?;

    let item = parsed.data.first().context("empty image response")?;
    let max_retries = req.max_retries.unwrap_or(api.config.max_retries);

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

struct PreparedImage {
    body: Value,
    ratio: String,
    size: String,
    input_record: Value,
}

fn prepare_image_body(api: &ApiClient, req: &ImageRequest) -> Result<PreparedImage> {
    let base = serde_json::to_value(ImageGenerationRequest {
        model: api.config.image_model.clone(),
        prompt: req.prompt.clone(),
        ratio: req.ratio.label(),
        size: req.size_tier.as_str().into(),
        extra_body: Some(ExtraBodyImage { image: None, response_format: Some("url".into()), seed: None }),
    })?;
    let mut explicit = req.explicit.clone();
    anyhow::ensure!(explicit.is_object(), "explicit image parameters must be an object");
    explicit["prompt"] = json!(req.prompt);
    if let Some(seed) = req.seed {
        explicit["extra_body"]["seed"] = json!(seed);
    }
    if !req.inputs.is_empty() {
        explicit["extra_body"]["image"] = json!(req.inputs);
    }
    let resolved = api
        .config
        .request_params
        .resolve(RequestKind::Image, &api.config.image_model, &base, &explicit)?;
    let mut body = resolved.body;
    if let Some(count) = body.get("n") {
        anyhow::ensure!(
            count.as_u64() == Some(1),
            "image n must be 1; use --count for batch generation"
        );
    }
    let ratio = AspectRatio::parse(body["ratio"].as_str().context("image ratio must be a string")?)?;
    validate_image_ratio(&ratio)?;
    let ratio_label = ratio.label();
    let size = ImageResolutionTier::parse(body["size"].as_str().context("image size must be a string")?)?
        .as_str()
        .to_string();
    let extra = body["extra_body"]
        .as_object_mut()
        .context("image extra_body must be an object")?;
    let fixed_seed = extra
        .get("seed")
        .filter(|value| !value.is_null())
        .map(|value| {
            let seed = value.as_u64().context("image seed must be an integer from 0 to 999")?;
            anyhow::ensure!(seed <= 999, "image seed must be an integer from 0 to 999");
            Ok::<u32, anyhow::Error>(seed as u32)
        })
        .transpose()?;
    anyhow::ensure!(
        req.batch_count <= 1 || fixed_seed.is_none(),
        "fixed image seed cannot be used with --count > 1; unset the configured seed for batch generation"
    );
    let seed = resolve_image_seed(fixed_seed)?;
    extra.insert("seed".into(), json!(seed));
    if let Some(format) = extra.get("response_format") {
        anyhow::ensure!(
            matches!(format.as_str(), Some("url" | "b64_json")),
            "image response_format must be url or b64_json"
        );
    }
    let inputs: Vec<String> = match extra.get("image") {
        Some(value) => {
            serde_json::from_value(value.clone()).context("image extra_body.image must be an array of strings")?
        }
        None => Vec::new(),
    };
    let parsed = parse_image_inputs(&inputs)?;
    let mut resolved_inputs = Vec::new();
    let mut image_urls = Vec::new();
    for raw in &parsed {
        let resolved = api.db.resolve_reference(raw)?;
        resolved_inputs.push(resolved.clone());
        let classified = classify_input(&resolved);
        image_urls.push(resolve_for_api(&classified, &api.client)?);
    }

    if !image_urls.is_empty() {
        extra.insert("image".into(), json!(image_urls));
    }
    body["ratio"] = json!(ratio_label);
    body["size"] = json!(size);
    log::debug!(
        "image generate ratio={ratio_label} size={size} seed={seed} inputs={} save_local={}",
        parsed.len(),
        req.save_local
    );
    let input_record = json!({
        "prompt": req.prompt,
        "ratio": ratio_label,
        "size": size,
        "seed": seed,
        "inputs": parsed,
        "resolved_inputs": resolved_inputs,
        "model": api.config.image_model,
        "request_body": crate::request_log::redact_credentials(&body),
        "request_config": {"hash":resolved.config_hash,"layers":resolved.layers},
    });
    Ok(PreparedImage { body, ratio: ratio_label, size, input_record })
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn request() -> ImageRequest {
        ImageRequest {
            prompt: "current prompt".into(),
            ratio: AspectRatio::parse("1:1").unwrap(),
            size_tier: ImageResolutionTier::parse("1K").unwrap(),
            inputs: Vec::new(),
            seed: None,
            explicit: json!({}),
            batch_count: 1,
            output_dir: None,
            save_local: false,
            max_retries: None,
            output_format: OutputFormat::Json,
            quiet: true,
        }
    }

    fn api() -> ApiClient {
        ApiClient {
            client: reqwest::blocking::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .unwrap(),
            config: crate::config::AppConfig::default(),
            api_key: "fixture-key".into(),
            db: crate::db::Database::open_memory().unwrap(),
            last_request_id: Default::default(),
        }
    }

    #[test]
    fn image_defaults_reach_http_body_and_history_without_losing_unspecified_fields() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            let header_end = loop {
                if let Some(index) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    break index + 4;
                }
                let read = stream.read(&mut buffer).unwrap();
                assert_ne!(read, 0);
                request.extend_from_slice(&buffer[..read]);
            };
            let headers = std::str::from_utf8(&request[..header_end]).unwrap();
            assert!(headers.starts_with("POST /v1/images/generations HTTP/1.1\r\n"));
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            while request.len() < header_end + length {
                let read = stream.read(&mut buffer).unwrap();
                assert_ne!(read, 0);
                request.extend_from_slice(&buffer[..read]);
            }
            let body: Value = serde_json::from_slice(&request[header_end..header_end + length]).unwrap();
            let response = r#"{"data":[{"url":"https://example.com/result.png"}]}"#;
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
            body
        });
        let mut api = api();
        api.config.base_url = format!("http://{address}");
        api.config.request_params=serde_json::from_value(json!({"image":{"body":{"size":"2K","ratio":"16:9","extra_body":{"seed":7,"vendor":{"keep":true,"changed":"configured","token":"image-private"}},"unknown_null":null}}})).unwrap();
        let mut req = request();
        req.explicit = json!({"ratio":"1:1","extra_body":{"seed":9,"vendor":{"changed":"explicit"}}});
        let result = generate_image(&api, req).unwrap();
        let body = server.join().unwrap();
        assert_eq!(body["ratio"], "1:1");
        assert_eq!(body["size"], "2K");
        assert_eq!(body["extra_body"]["seed"], 9);
        assert_eq!(
            body["extra_body"]["vendor"],
            json!({"keep":true,"changed":"explicit","token":"image-private"})
        );
        assert!(body.get("unknown_null").unwrap().is_null());
        assert_eq!(result.size, "2K");
        assert_eq!(result.ratio, "1:1");
        let record = api.db.get_generation(result.generation_id.unwrap()).unwrap();
        assert_eq!(
            record.input_json["request_body"],
            crate::request_log::redact_credentials(&body)
        );
        assert!(!record.input_json.to_string().contains("image-private"));
        assert_eq!(record.input_json["seed"], 9);
        let run = api.db.get_request_run(api.last_request_id.get().unwrap()).unwrap();
        assert_eq!(run["status"], "succeeded");
        assert_eq!(run["http_status"], 200);
        assert!(run["config_layers"].as_array().unwrap().contains(&json!("explicit")));
        assert!(!run.to_string().contains("image-private"));
    }

    #[test]
    fn image_configured_seed_is_fixed_and_participates_in_batch_validation() {
        let mut api = api();
        api.config.request_params =
            serde_json::from_value(json!({"image":{"body":{"extra_body":{"seed":42}}}})).unwrap();
        let mut req = request();
        assert_eq!(prepare_image_body(&api, &req).unwrap().body["extra_body"]["seed"], 42);
        req.batch_count = 2;
        assert!(prepare_image_body(&api, &req).is_err());
        api.config.request_params = Default::default();
        assert!(prepare_image_body(&api, &req).is_ok());
    }

    #[test]
    fn explicit_empty_image_array_and_cleared_seed_preserve_other_defaults() {
        let mut api = api();
        api.config.request_params=serde_json::from_value(json!({"image":{"body":{"size":"2K","extra_body":{"image":["https://example.com/configured.png"],"seed":42,"vendor":true}}}})).unwrap();
        let mut req = request();
        req.explicit = json!({"extra_body":{"image":[],"seed":null}});
        req.batch_count = 2;
        let prepared = prepare_image_body(&api, &req).unwrap();
        assert_eq!(prepared.body["extra_body"]["image"], json!([]));
        assert_eq!(prepared.body["extra_body"]["vendor"], true);
        assert_eq!(prepared.body["size"], "2K");
        assert!(prepared.body["extra_body"]["seed"].as_u64().unwrap() <= 999);
    }

    #[test]
    fn seed_range() {
        assert!(resolve_image_seed(Some(999)).is_ok());
        assert!(resolve_image_seed(Some(1000)).is_err());
        let s = resolve_image_seed(None).unwrap();
        assert!(s <= 999);
    }
}
