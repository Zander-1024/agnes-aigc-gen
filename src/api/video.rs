use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use reqwest::Url;
use serde::Serialize;
use serde_json::{Value, json};

use crate::api::ApiClient;
use crate::api::deliver::{DeliverOptions, DeliverPayload, deliver_generation};
use crate::api::error;
use crate::api::image::resolve_image_seed;
use crate::api::types::{ExtraBodyVideo, VideoCreateRequest, VideoStatus, VideoTaskResponse};
use crate::db::VideoTaskRecord;
use crate::media::input::ImageInput;
use crate::media::input::{ensure_same_ratio, image_dimensions_from_bytes, load_image_bytes};
use crate::media::{classify_input, parse_image_inputs};
use crate::output::{GenerationResult, OutputFormat, download_with_retry, infer_ext_from_url};
use crate::ratio::{AspectRatio, resolve_video_timing, video_dimensions};
use crate::retry::{backoff_delay, is_retryable_http_status};

const POLL_EARLY_PHASE: Duration = Duration::from_secs(120);
const POLL_INTERVAL_EARLY: Duration = Duration::from_secs(30);
const POLL_INTERVAL_LATE: Duration = Duration::from_secs(15);

pub struct VideoRequest {
    pub prompt: String,
    pub negative_prompt: Option<String>,
    pub seed: Option<u32>,
    pub ratio: AspectRatio,
    pub duration: f64,
    pub frame_rate: u32,
    pub images: Vec<String>,
    pub task_id: Option<String>,
    /// Submit task and return immediately without polling.
    pub async_mode: bool,
    pub output_dir: Option<String>,
    pub save_local: bool,
    pub max_retries: Option<u32>,
    pub output_format: OutputFormat,
    pub quiet: bool,
}

#[derive(Debug, Serialize)]
pub struct VideoTaskSubmitResult {
    /// Local short id for `task show N` / `task wait N`.
    pub id: i64,
    #[serde(rename = "async")]
    pub is_async: bool,
    pub task_id: String,
    pub status: String,
    pub phase: String,
}

/// Output-side parameters for polling an existing task and recording the result.
#[derive(Debug, Clone)]
pub struct FinalizeOutput {
    pub prompt: String,
    pub ratio_label: String,
    pub size: String,
    pub save_local: bool,
    pub output_dir: Option<String>,
    pub max_retries: Option<u32>,
    pub output_format: OutputFormat,
    pub quiet: bool,
    pub input_record: Option<serde_json::Value>,
}

impl FinalizeOutput {
    pub fn from_request(
        req: &VideoRequest,
        ratio_label: String,
        size: String,
        input_record: Option<serde_json::Value>,
    ) -> Self {
        Self {
            prompt: req.prompt.clone(),
            ratio_label,
            size,
            save_local: req.save_local,
            output_dir: req.output_dir.clone(),
            max_retries: req.max_retries,
            output_format: req.output_format,
            quiet: req.quiet,
            input_record,
        }
    }

    pub fn for_wait(save_local: bool, output_format: OutputFormat) -> Self {
        Self {
            prompt: String::new(),
            ratio_label: "-".into(),
            size: "-".into(),
            save_local,
            output_dir: None,
            max_retries: None,
            output_format,
            quiet: false,
            input_record: None,
        }
    }
}

pub fn generate_video(api: &ApiClient, req: VideoRequest) -> Result<GenerationResult> {
    if let Some(ref query_id) = req.task_id {
        return poll_and_finalize(
            api,
            query_id,
            &FinalizeOutput::from_request(&req, "-".into(), "-".into(), None),
        );
    }

    let (num_frames, actual_duration) = resolve_video_timing(req.duration, req.frame_rate)?;
    let seed = match req.seed {
        Some(s) => Some(resolve_image_seed(Some(s))?),
        None => None,
    };

    let inputs_raw = parse_image_inputs(&req.images)?;

    let resolved: Vec<String> = inputs_raw
        .iter()
        .map(|s| api.db.resolve_reference(s))
        .collect::<Result<_>>()?;

    let mut input_record = json!({
        "prompt": req.prompt,
        "negative_prompt": req.negative_prompt,
        "seed": seed,
        "ratio": req.ratio.label(),
        "duration": req.duration,
        "actual_duration": actual_duration,
        "frame_rate": req.frame_rate,
        "num_frames": num_frames,
        "images": req.images,
        "resolved_images": resolved,
        "model": api.config.video_model,
    });

    let (dims, frame_urls) = if resolved.is_empty() {
        (video_dimensions(&req.ratio), Vec::new())
    } else {
        let (dims, urls) = video_frame_urls(&resolved, &api.client)?;
        (dims, urls)
    };

    log::debug!(
        "video create size={}x{} frames={num_frames} duration={actual_duration:.3}s frame_rate={} seed={seed:?} input_frames={}",
        dims.width,
        dims.height,
        req.frame_rate,
        frame_urls.len()
    );
    match frame_urls.len() {
        0 => log::debug!("video mode: text-to-video"),
        1 => log::debug!("video mode: image-to-video"),
        n => log::debug!("video mode: multi-frame ({n} images)"),
    }

    let mut body = VideoCreateRequest {
        model: api.config.video_model.clone(),
        prompt: req.prompt.clone(),
        image: None,
        height: Some(dims.height),
        width: Some(dims.width),
        num_frames: Some(num_frames),
        frame_rate: Some(req.frame_rate),
        negative_prompt: req.negative_prompt.clone(),
        seed,
        extra_body: None,
    };

    apply_video_frames(&mut body, frame_urls);

    let resp = api.post_json("videos", &body)?;
    let resp = ApiClient::ensure_success(resp, "video task creation")?;
    let created: VideoTaskResponse = resp.json().context("parse video create response")?;
    let query_ids = created.query_ids();
    let query_id = query_ids.first().cloned().context("missing video_id/task_id")?;
    record_video_query_ids(&mut input_record, &query_ids, &created);
    log::debug!("video task created, query_id={query_id}");

    api.db.insert_video_task(
        &query_id,
        created.status.as_str(),
        Some(&req.prompt),
        Some(&input_record),
        created.progress,
    )?;

    if req.async_mode {
        let record = api.db.get_video_task(&query_id)?;
        let submit = VideoTaskSubmitResult {
            id: record.id,
            is_async: true,
            task_id: query_id.clone(),
            status: created.status.to_string(),
            phase: VideoTaskRecord::phase_from_status(&created.status).to_string(),
        };
        if !req.quiet {
            print_submit_result(&submit, req.output_format)?;
        }
        return Ok(GenerationResult {
            kind: "video".into(),
            ratio: req.ratio.label(),
            size: dims.size_string(),
            uri: String::new(),
            asset_uri: None,
            generation_id: Some(record.id),
        });
    }

    poll_and_finalize(
        api,
        &query_id,
        &FinalizeOutput::from_request(&req, req.ratio.label(), dims.size_string(), Some(input_record)),
    )
}

fn print_submit_result(result: &VideoTaskSubmitResult, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(result)?),
        OutputFormat::Plain => {
            println!("async=true");
            println!("id={}", result.id);
            println!("task_id={}", result.task_id);
            println!("status={}", result.status);
            println!("phase={}", result.phase);
        }
    }
    Ok(())
}

fn poll_interval(elapsed: Duration) -> Duration {
    if elapsed < POLL_EARLY_PHASE {
        POLL_INTERVAL_EARLY
    } else {
        POLL_INTERVAL_LATE
    }
}

fn record_video_query_ids(input_record: &mut Value, query_ids: &[String], created: &VideoTaskResponse) {
    let Some(obj) = input_record.as_object_mut() else {
        return;
    };
    obj.insert("video_query_ids".to_string(), json!(query_ids));
    if let Some(ref video_id) = created.video_id {
        obj.insert("video_id".to_string(), json!(video_id));
    }
    if let Some(ref id) = created.id {
        obj.insert("response_id".to_string(), json!(id));
    }
    if let Some(ref task_id) = created.task_id {
        obj.insert("task_id".to_string(), json!(task_id));
    }
}

/// Single GET for a video task (no polling loop).
pub fn fetch_video_task_once(api: &ApiClient, query_id: &str) -> Result<VideoTaskResponse> {
    let resp = get_video_query(api, query_id)?;
    let resp = ApiClient::ensure_success(resp, "fetch video task")?;
    resp.json().context("parse video task")
}

fn get_video_query(api: &ApiClient, query_id: &str) -> Result<reqwest::blocking::Response> {
    if is_video_id(query_id) {
        let url = video_id_result_url(&api.config.base_url, &api.config.video_model, query_id)?;
        return api.get_json_url(&url);
    }
    let path = format!("videos/{query_id}");
    api.get_json(&path)
}

fn is_video_id(query_id: &str) -> bool {
    query_id.trim_start().starts_with("video_")
}

fn video_id_result_url(base_url: &str, video_model: &str, video_id: &str) -> Result<String> {
    let mut url = Url::parse(base_url.trim()).context("parse base_url")?;
    let path = url.path().trim_end_matches('/');
    let root_path = path.strip_suffix("/v1").unwrap_or(path).trim_end_matches('/');
    let agnesapi_path = if root_path.is_empty() {
        "/agnesapi".to_string()
    } else {
        format!("{root_path}/agnesapi")
    };
    url.set_path(&agnesapi_path);
    url.set_query(None);
    url.query_pairs_mut()
        .append_pair("video_id", video_id)
        .append_pair("model_name", video_model);
    Ok(url.into())
}

/// Refresh one task from the API and persist status in SQLite.
pub fn refresh_video_task(api: &ApiClient, query_id: &str) -> Result<VideoTaskRecord> {
    let task = fetch_video_task_once(api, query_id)?;
    persist_task_from_response(api, query_id, &task)
}

pub fn wait_video_task(
    api: &ApiClient,
    query_id: &str,
    save_local: bool,
    output_format: OutputFormat,
) -> Result<GenerationResult> {
    poll_and_finalize(api, query_id, &FinalizeOutput::for_wait(save_local, output_format))
}

fn persist_task_from_response(api: &ApiClient, query_id: &str, task: &VideoTaskResponse) -> Result<VideoTaskRecord> {
    let remote = task.result_url();
    let asset_id = if task.status == VideoStatus::Completed {
        if let Some(ref url) = remote {
            resolve_or_create_video_asset(api, query_id, url)?
        } else {
            None
        }
    } else {
        None
    };

    match api.db.get_video_task(query_id) {
        Ok(_) => api.db.update_video_task(
            query_id,
            task.status.as_str(),
            task.progress,
            remote.as_deref(),
            asset_id.as_deref(),
            task.error.as_ref(),
        ),
        Err(_) => {
            api.db
                .insert_video_task(query_id, task.status.as_str(), None, None, task.progress)?;
            api.db.update_video_task(
                query_id,
                task.status.as_str(),
                task.progress,
                remote.as_deref(),
                asset_id.as_deref(),
                task.error.as_ref(),
            )
        }
    }
}

fn resolve_or_create_video_asset(api: &ApiClient, query_id: &str, url: &str) -> Result<Option<String>> {
    if let Ok(existing) = api.db.get_video_task(query_id)
        && let Some(ref asset_uri) = existing.asset_uri
        && let Some(id) = asset_uri.strip_prefix("asset://")
    {
        return Ok(Some(id.to_string()));
    }
    let asset = api.db.insert_asset("video", url, None, None)?;
    Ok(Some(asset.id))
}

pub fn poll_video_task(api: &ApiClient, query_id: &str) -> Result<VideoTaskResponse> {
    let started = Instant::now();
    let mut http_attempt = 0u32;
    loop {
        let resp = get_video_query(api, query_id)?;
        let status = resp.status();
        if !status.is_success() {
            if is_retryable_http_status(status) && http_attempt < api.config.max_retries {
                http_attempt += 1;
                let wait = backoff_delay(http_attempt);
                log::debug!(
                    "video query {query_id} poll: {status} retry {http_attempt}/{} in {}ms",
                    api.config.max_retries,
                    wait.as_millis()
                );
                thread::sleep(wait);
                continue;
            }
            let body = resp.text().unwrap_or_default();
            return error::http_failure(status, body, "poll video");
        }
        http_attempt = 0;
        let task: VideoTaskResponse = resp.json().context("parse video task")?;
        let _ = persist_task_from_response(api, query_id, &task);
        match &task.status {
            VideoStatus::Completed => {
                log::debug!("video query {query_id} completed");
                return Ok(task);
            }
            VideoStatus::Failed => {
                log::debug!("video query {query_id} failed: {:?}", task.error);
                bail!("video task failed: {:?}", task.error);
            }
            status => {
                let wait = poll_interval(started.elapsed());
                log::debug!(
                    "video query {query_id} status={}, next poll in {}s",
                    status,
                    wait.as_secs()
                );
                thread::sleep(wait);
            }
        }
    }
}

fn poll_and_finalize(api: &ApiClient, query_id: &str, output: &FinalizeOutput) -> Result<GenerationResult> {
    let task = poll_video_task(api, query_id)?;
    let remote = task.result_url().context("completed task missing video url")?;

    let max_retries = output.max_retries.unwrap_or(api.config.max_retries);
    let uri = if output.save_local {
        log::debug!("downloading video to local output dir");
        let output_dir = api.config.resolved_output_dir()?;
        let out_dir = output
            .output_dir
            .as_ref()
            .map(|d| crate::config::expand_tilde(d))
            .transpose()?
            .unwrap_or(output_dir);
        let ext = infer_ext_from_url(&remote, "mp4");
        let path = download_with_retry(&api.client, &remote, &out_dir, &ext, max_retries)?;
        path.display().to_string()
    } else {
        remote.clone()
    };

    let _ = api
        .db
        .update_video_task(query_id, task.status.as_str(), task.progress, Some(&remote), None, None);

    let prompt = if output.prompt.is_empty() {
        None
    } else {
        Some(output.prompt.as_str())
    };
    let deliver_options = DeliverOptions { output_format: output.output_format, quiet: output.quiet };

    let result = deliver_generation(
        api,
        DeliverPayload {
            kind: "video",
            prompt,
            ratio: &output.ratio_label,
            size: &output.size,
            remote_url: &remote,
            uri,
            input_record: output.input_record.as_ref(),
        },
        &deliver_options,
    )?;

    if let Some(asset_uri) = &result.asset_uri
        && let Some(id) = asset_uri.strip_prefix("asset://")
    {
        let _ = api.db.update_video_task(
            query_id,
            task.status.as_str(),
            task.progress,
            Some(&remote),
            Some(id),
            None,
        );
    }

    Ok(result)
}

fn apply_video_frames(body: &mut VideoCreateRequest, frame_urls: Vec<String>) {
    match frame_urls.len() {
        0 => {}
        1 => body.image = Some(frame_urls[0].clone()),
        _ => body.extra_body = Some(ExtraBodyVideo { image: Some(frame_urls) }),
    }
}

fn video_frame_urls(
    resolved: &[String],
    client: &reqwest::blocking::Client,
) -> Result<(crate::ratio::Dimensions, Vec<String>)> {
    let classified: Vec<_> = resolved.iter().map(|s| classify_input(s)).collect();
    let urls: Vec<String> = classified
        .iter()
        .zip(resolved)
        .map(|(input, raw)| match input {
            ImageInput::Url(u) => Ok(u.clone()),
            _ => bail!(
                "video -i/--image requires HTTPS URL or asset:// (remote URL); \
                 unsupported input {raw:?}. Generate an image first and pass asset:// or a public URL. \
                 Local paths, base64, and data URIs are not supported for video"
            ),
        })
        .collect::<Result<_>>()?;

    let mut dims = Vec::new();
    for input in &classified {
        let bytes = load_image_bytes(input, client)?;
        dims.push(image_dimensions_from_bytes(&bytes)?);
    }
    ensure_same_ratio(&dims)?;
    let ratio = AspectRatio::from_dimensions(dims[0].0, dims[0].1);
    let target = video_dimensions(&ratio);
    Ok((target, urls))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_interval_phases() {
        assert_eq!(poll_interval(Duration::from_secs(0)), POLL_INTERVAL_EARLY);
        assert_eq!(poll_interval(Duration::from_secs(119)), POLL_INTERVAL_EARLY);
        assert_eq!(poll_interval(Duration::from_secs(120)), POLL_INTERVAL_LATE);
        assert_eq!(poll_interval(Duration::from_secs(300)), POLL_INTERVAL_LATE);
    }

    #[test]
    fn apply_video_frames_no_mode() {
        let mut body = VideoCreateRequest {
            model: "agnes-video-v2.0".into(),
            prompt: "test".into(),
            image: None,
            height: None,
            width: None,
            num_frames: None,
            frame_rate: None,
            negative_prompt: None,
            seed: None,
            extra_body: None,
        };
        apply_video_frames(&mut body, vec!["https://a/1.png".into(), "https://a/2.png".into()]);
        assert!(body.image.is_none());
        let extra = body.extra_body.as_ref().unwrap();
        assert_eq!(extra.image.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn video_id_result_url_uses_agnesapi_endpoint() {
        let url = video_id_result_url("https://apihub.agnes-ai.com/v1", "agnes-video-v2.0", "video_abc==").unwrap();

        assert_eq!(
            url,
            "https://apihub.agnes-ai.com/agnesapi?video_id=video_abc%3D%3D&model_name=agnes-video-v2.0"
        );
    }
}
