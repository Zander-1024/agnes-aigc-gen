use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use reqwest::Url;
use serde::Serialize;
use serde_json::{Value, json};

use crate::api::ApiClient;
use crate::api::deliver::{DeliverOptions, DeliverPayload, deliver_generation};
use crate::api::error;
use crate::api::types::{VideoCreateRequest, VideoReference, VideoStatus, VideoTaskResponse};
use crate::db::VideoTaskRecord;
use crate::media::parse_image_inputs;
use crate::output::{GenerationResult, OutputFormat, download_with_retry, infer_ext_from_url};
use crate::ratio::{AspectRatio, validate_video_duration, validate_video_ratio, video_size_options};
use crate::retry::{backoff_delay, is_retryable_http_status};

const POLL_EARLY_PHASE: Duration = Duration::from_secs(120);
const POLL_INTERVAL_EARLY: Duration = Duration::from_secs(30);
const POLL_INTERVAL_LATE: Duration = Duration::from_secs(15);

pub struct VideoRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub size: String,
    pub seed: Option<u32>,
    pub ratio: AspectRatio,
    pub duration: f64,
    pub images: Vec<String>,
    pub first_frame: Option<String>,
    pub last_frame: Option<String>,
    pub audios: Vec<String>,
    pub videos: Vec<VideoReference>,
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

    let model = req.model.as_deref().unwrap_or(&api.config.video_model);
    let mut body = build_video_body(&req, model)?;
    resolve_video_media(api, &mut body)?;
    let mut input_record = serde_json::to_value(&body)?;
    log::debug!(
        "video create model={} mode={} size={} duration={}s ratio={}",
        body.model,
        body.mode,
        body.size,
        body.seconds,
        body.aspect_ratio
    );

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
            size: body.size.clone(),
            uri: String::new(),
            asset_uri: None,
            generation_id: Some(record.id),
        });
    }

    poll_and_finalize(
        api,
        &query_id,
        &FinalizeOutput::from_request(&req, req.ratio.label(), body.size, Some(input_record)),
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
        let record = api.db.get_video_task(query_id).ok();
        let model = recorded_video_model(record.as_ref()).unwrap_or(&api.config.video_model);
        let url = video_id_result_url(&api.config.base_url, model, query_id)?;
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

fn recorded_video_model(record: Option<&VideoTaskRecord>) -> Option<&str> {
    record?.input_json.as_ref()?.get("model")?.as_str()
}

fn build_video_body(req: &VideoRequest, model: &str) -> Result<VideoCreateRequest> {
    anyhow::ensure!(
        matches!(model, "agnes-video-2.5" | "agnes-video-2.5-flash"),
        "unsupported video model {model:?}; use agnes-video-2.5 or agnes-video-2.5-flash"
    );
    let seconds = validate_video_duration(req.duration)?.to_string();
    validate_video_ratio(&req.ratio)?;
    let size = req.size.trim().to_ascii_uppercase();
    anyhow::ensure!(
        video_size_options(model).contains(&size.as_str()),
        "unsupported size {size:?} for {model}; supported: {}",
        video_size_options(model).join(", ")
    );
    let images = parse_image_inputs(&req.images)?;
    let audios = parse_image_inputs(&req.audios)?;
    let has_frames = req.first_frame.is_some() || req.last_frame.is_some();
    let inferred_mode = if has_frames || (images.len() == 1 && audios.is_empty() && req.videos.is_empty()) {
        "keyframe"
    } else if !images.is_empty() || !audios.is_empty() || !req.videos.is_empty() {
        "reference"
    } else {
        "text"
    };
    let mode = req.mode.as_deref().unwrap_or(inferred_mode);
    let mut body = VideoCreateRequest {
        model: model.into(),
        prompt: req.prompt.clone(),
        mode: mode.into(),
        seconds,
        size,
        aspect_ratio: req.ratio.label(),
        n: 1,
        first_frame: req.first_frame.clone(),
        last_frame: req.last_frame.clone(),
        images,
        audios,
        videos: req.videos.clone(),
        seed: req.seed,
    };
    match mode {
        "text" => anyhow::ensure!(
            !has_frames && body.images.is_empty() && body.audios.is_empty() && body.videos.is_empty(),
            "text mode does not accept reference media or first/last frames"
        ),
        "keyframe" => {
            anyhow::ensure!(
                body.audios.is_empty() && body.videos.is_empty(),
                "keyframe mode does not accept audio or video references"
            );
            if !body.images.is_empty() {
                anyhow::ensure!(
                    body.images.len() == 1 && body.first_frame.is_none(),
                    "keyframe mode accepts one -i image as the first frame; use --first-frame/--last-frame for explicit frames"
                );
                body.first_frame = body.images.pop();
            }
            anyhow::ensure!(
                body.first_frame.is_some() || body.last_frame.is_some(),
                "keyframe mode requires a first or last frame"
            );
        }
        "reference" => {
            anyhow::ensure!(!has_frames, "reference mode does not accept first/last frames");
            anyhow::ensure!(
                !body.images.is_empty() || !body.audios.is_empty() || !body.videos.is_empty(),
                "reference mode requires images, audio, or video"
            );
            let max_images = if model.ends_with("-flash") { 5 } else { 8 };
            anyhow::ensure!(
                body.images.len() <= max_images,
                "{model} accepts at most {max_images} reference images"
            );
            anyhow::ensure!(body.audios.len() <= 3, "reference mode accepts at most 3 audio clips");
            let max_videos = if model.ends_with("-flash") { 0 } else { 1 };
            anyhow::ensure!(
                body.videos.len() <= max_videos,
                "{model} accepts at most {max_videos} reference videos"
            );
            anyhow::ensure!(
                body.images.len() + body.audios.len() + body.videos.len() <= 12,
                "reference mode accepts at most 12 media files"
            );
        }
        _ => bail!("unsupported video mode {mode:?}; use text, keyframe, or reference"),
    }
    for video in &body.videos {
        if let Some(start) = video.start_seconds {
            anyhow::ensure!(
                start.is_finite() && start >= 0.0,
                "video start_seconds must be a finite non-negative number"
            );
        }
    }
    Ok(body)
}

fn resolve_video_media(api: &ApiClient, body: &mut VideoCreateRequest) -> Result<()> {
    for frame in [&mut body.first_frame, &mut body.last_frame].into_iter().flatten() {
        *frame = resolve_video_media_url(api, frame)?;
    }
    for url in body.images.iter_mut().chain(&mut body.audios) {
        *url = resolve_video_media_url(api, url)?;
    }
    for video in &mut body.videos {
        video.url = resolve_video_media_url(api, &video.url)?;
    }
    Ok(())
}

fn resolve_video_media_url(api: &ApiClient, raw: &str) -> Result<String> {
    let resolved = api.db.resolve_reference(raw)?;
    validate_video_media_url(&resolved)?;
    Ok(resolved)
}

fn validate_video_media_url(resolved: &str) -> Result<()> {
    let url = Url::parse(resolved).context("video media requires an HTTPS URL or asset:// reference")?;
    anyhow::ensure!(
        url.scheme() == "https" && url.host_str().is_some(),
        "video media requires an HTTPS URL or asset:// reference; local paths, base64, and data URIs are not supported"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> VideoRequest {
        VideoRequest {
            prompt: "test".into(),
            model: None,
            mode: None,
            size: "720P".into(),
            seed: Some(1101),
            ratio: AspectRatio { w: 16, h: 9 },
            duration: 5.0,
            images: vec![],
            first_frame: None,
            last_frame: None,
            audios: vec![],
            videos: vec![],
            task_id: None,
            async_mode: true,
            output_dir: None,
            save_local: false,
            max_retries: None,
            output_format: OutputFormat::Json,
            quiet: true,
        }
    }

    #[test]
    fn poll_interval_phases() {
        assert_eq!(poll_interval(Duration::from_secs(0)), POLL_INTERVAL_EARLY);
        assert_eq!(poll_interval(Duration::from_secs(119)), POLL_INTERVAL_EARLY);
        assert_eq!(poll_interval(Duration::from_secs(120)), POLL_INTERVAL_LATE);
    }

    #[test]
    fn video_id_result_url_uses_agnesapi_endpoint() {
        let url =
            video_id_result_url("https://apihub.agnes-ai.com/v1", "agnes-video-2.5-flash", "video_abc==").unwrap();
        assert_eq!(
            url,
            "https://apihub.agnes-ai.com/agnesapi?video_id=video_abc%3D%3D&model_name=agnes-video-2.5-flash"
        );
    }

    #[test]
    fn polling_uses_model_saved_with_the_task() {
        let record: VideoTaskRecord = serde_json::from_value(json!({
            "id": 1,
            "task_id": "video_123",
            "status": "queued",
            "phase": "processing",
            "prompt": "test",
            "input_json": {"model": "agnes-video-2.5-flash"},
            "created_at": "2026-10-31",
            "updated_at": "2026-10-31"
        }))
        .unwrap();
        let model = recorded_video_model(Some(&record)).unwrap();
        let url = video_id_result_url("https://example.com/v1", model, &record.task_id).unwrap();
        assert!(url.ends_with("model_name=agnes-video-2.5-flash"));
        assert!(recorded_video_model(None).is_none());
    }

    #[test]
    fn video_request_matches_current_protocol() {
        let body = build_video_body(&request(), "agnes-video-2.5").unwrap();
        let value = serde_json::to_value(body).unwrap();
        assert_eq!(value["seconds"], "5");
        assert_eq!(value["mode"], "text");
        assert_eq!(value["aspect_ratio"], "16:9");
        assert_eq!(value["size"], "720P");
        assert_eq!(value["seed"], 1101);
        for legacy in ["width", "height", "num_frames", "frame_rate", "negative_prompt", "extra_body"] {
            assert!(value.get(legacy).is_none());
        }
    }

    #[test]
    fn one_image_defaults_to_first_frame_and_explicit_reference_is_supported() {
        let mut req = request();
        req.images = vec!["https://example.com/image.png".into()];
        let body = build_video_body(&req, "agnes-video-2.5").unwrap();
        assert_eq!(body.mode, "keyframe");
        assert_eq!(body.first_frame.as_deref(), Some("https://example.com/image.png"));
        assert!(body.images.is_empty());
        req.mode = Some("reference".into());
        let body = build_video_body(&req, "agnes-video-2.5").unwrap();
        assert_eq!(body.images.len(), 1);
        assert!(body.first_frame.is_none());
    }

    #[test]
    fn keyframe_and_multimodal_reference_payloads_use_top_level_fields() {
        let mut req = request();
        req.last_frame = Some("https://example.com/last.png".into());
        let value = serde_json::to_value(build_video_body(&req, "agnes-video-2.5").unwrap()).unwrap();
        assert_eq!(value["mode"], "keyframe");
        assert_eq!(value["last_frame"], "https://example.com/last.png");
        assert!(value.get("first_frame").is_none());
        assert!(value.get("images").is_none());

        req.last_frame = None;
        req.images = vec!["https://example.com/image.png".into(); 2];
        req.audios = vec!["https://example.com/audio.mp3".into()];
        req.videos = vec![VideoReference {
            url: "https://example.com/input.mp4".into(),
            start_seconds: Some(35.5),
            require_audio: true,
        }];
        let value = serde_json::to_value(build_video_body(&req, "agnes-video-2.5").unwrap()).unwrap();
        assert_eq!(value["mode"], "reference");
        assert_eq!(value["images"].as_array().unwrap().len(), 2);
        assert_eq!(value["audios"][0], "https://example.com/audio.mp3");
        assert_eq!(value["videos"][0]["start_seconds"], 35.5);
        assert_eq!(value["videos"][0]["require_audio"], true);
        assert_eq!(value["n"], 1);
        assert!(value.get("extra_body").is_none());
    }

    #[test]
    fn reference_media_count_and_start_time_limits() {
        let mut req = request();
        req.images = vec!["https://example.com/image.png".into(); 8];
        req.audios = vec!["https://example.com/audio.mp3".into(); 3];
        req.videos = vec![VideoReference {
            url: "https://example.com/input.mp4".into(),
            start_seconds: Some(0.0),
            require_audio: false,
        }];
        assert!(build_video_body(&req, "agnes-video-2.5").is_ok());
        req.images.push("https://example.com/extra.png".into());
        assert!(build_video_body(&req, "agnes-video-2.5").is_err());
        req.images.truncate(5);
        req.videos.clear();
        assert!(build_video_body(&req, "agnes-video-2.5-flash").is_ok());
        req.audios.push("https://example.com/extra.mp3".into());
        assert!(build_video_body(&req, "agnes-video-2.5-flash").is_err());
        req.audios.clear();
        req.videos = vec![VideoReference {
            url: "https://example.com/input.mp4".into(),
            start_seconds: Some(-1.0),
            require_audio: false,
        }];
        assert!(build_video_body(&req, "agnes-video-2.5").is_err());
    }

    #[test]
    fn video_media_requires_https_after_asset_resolution() {
        assert!(validate_video_media_url("https://example.com/input.png").is_ok());
        for invalid in ["http://example.com/input.png", "./input.png", "data:image/png;base64,abcd", "base64:abcd"] {
            assert!(validate_video_media_url(invalid).is_err());
        }
    }

    #[test]
    fn rejects_invalid_modes_and_flash_limits_before_submission() {
        let mut req = request();
        req.mode = Some("reference".into());
        assert!(build_video_body(&req, "agnes-video-2.5").is_err());
        req.images = vec!["https://example.com/image.png".into(); 6];
        assert!(build_video_body(&req, "agnes-video-2.5").is_ok());
        assert!(build_video_body(&req, "agnes-video-2.5-flash").is_err());
        req.images.clear();
        req.videos = vec![VideoReference {
            url: "https://example.com/video.mp4".into(),
            start_seconds: Some(35.0),
            require_audio: false,
        }];
        assert!(build_video_body(&req, "agnes-video-2.5").is_ok());
        assert!(build_video_body(&req, "agnes-video-2.5-flash").is_err());
        req.videos.clear();
        req.mode = Some("text".into());
        req.size = "1080P".into();
        assert!(build_video_body(&req, "agnes-video-2.5-flash").is_err());
        req.size = "720P".into();
        req.first_frame = Some("https://example.com/first.png".into());
        assert!(build_video_body(&req, "agnes-video-2.5").is_err());
    }
}
