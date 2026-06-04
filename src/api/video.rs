use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use log;
use reqwest::StatusCode;
use serde_json::json;

use crate::api::ApiClient;
use crate::api::types::{
    ExtraBodyImage, ExtraBodyVideo, ImageGenerationRequest, ImageGenerationResponse, VideoCreateRequest,
    VideoTaskResponse,
};
use crate::logging;
use crate::media::input::ImageInput;
use crate::media::input::{ensure_same_ratio, image_dimensions_from_bytes, load_image_bytes};
use crate::media::{classify_input, parse_image_inputs, prepare_video_frames};
use crate::output::{GenerationResult, OutputFormat, download_with_retry, infer_ext_from_url, retry};
use crate::ratio::{AspectRatio, resolve_video_timing, video_dimensions};

const POLL_EARLY_PHASE: Duration = Duration::from_secs(120);
const POLL_INTERVAL_EARLY: Duration = Duration::from_secs(30);
const POLL_INTERVAL_LATE: Duration = Duration::from_secs(15);

pub struct VideoRequest {
    pub prompt: String,
    pub ratio: AspectRatio,
    pub duration: f64,
    pub frame_rate: u32,
    pub images: Vec<String>,
    pub task_id: Option<String>,
    pub output_dir: Option<String>,
    pub save_local: bool,
    pub max_retries: Option<u32>,
    pub output_format: OutputFormat,
    pub quiet: bool,
}

pub fn generate_video(api: &ApiClient, req: VideoRequest) -> Result<GenerationResult> {
    if let Some(ref task_id) = req.task_id {
        return poll_and_finalize(api, task_id, &req, None);
    }

    let (num_frames, actual_duration) = resolve_video_timing(req.duration, req.frame_rate)?;

    let inputs_raw = parse_image_inputs(&req.images)?;

    let resolved: Vec<String> = inputs_raw
        .iter()
        .map(|s| api.db.resolve_reference(s))
        .collect::<Result<_>>()?;

    let classified: Vec<_> = resolved.iter().map(|s| classify_input(s)).collect();

    let input_record = json!({
        "prompt": req.prompt,
        "ratio": req.ratio.label(),
        "duration": req.duration,
        "actual_duration": actual_duration,
        "frame_rate": req.frame_rate,
        "num_frames": num_frames,
        "images": req.images,
        "resolved_images": resolved,
        "model": api.config.video_model,
    });

    let (dims, frame_urls) = if classified.is_empty() {
        (video_dimensions(&req.ratio), Vec::new())
    } else if classified.iter().all(|i| matches!(i, ImageInput::Url(_))) {
        let (dims, _, urls) = video_frame_urls(&classified, &api.client)?;
        (dims, urls)
    } else {
        let (frames, _, dims) = prepare_video_frames(&classified, &api.client)?;
        let mut urls = Vec::with_capacity(frames.len());
        for frame in frames {
            urls.push(stage_frame_as_url(api, &frame.payload, &dims)?);
        }
        (dims, urls)
    };

    log::debug!(
        "video create size={}x{} frames={num_frames} duration={actual_duration:.3}s frame_rate={} input_frames={}",
        dims.width,
        dims.height,
        req.frame_rate,
        frame_urls.len()
    );
    match frame_urls.len() {
        0 => log::debug!("video mode: text-to-video"),
        1 => log::debug!("video mode: image-to-video"),
        2 => log::debug!("video mode: keyframes"),
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
        extra_body: None,
    };

    match frame_urls.len() {
        0 => {}
        1 => body.image = Some(frame_urls[0].clone()),
        2 => {
            body.extra_body = Some(ExtraBodyVideo { image: Some(frame_urls), mode: Some("keyframes".into()) });
        }
        _ => {
            body.extra_body = Some(ExtraBodyVideo { image: Some(frame_urls), mode: None });
        }
    }

    let resp = api.post_json("videos", &body)?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        logging::log_response(status.as_u16(), &text);
        bail!("video task creation failed ({status}): {text}");
    }
    let created: VideoTaskResponse = resp.json().context("parse video create response")?;
    let task_id = created.task_id().context("missing task_id")?;
    log::debug!("video task created: {task_id}");
    poll_and_finalize(api, &task_id, &req, Some(input_record))
}

fn poll_interval(elapsed: Duration) -> Duration {
    if elapsed < POLL_EARLY_PHASE {
        POLL_INTERVAL_EARLY
    } else {
        POLL_INTERVAL_LATE
    }
}

pub fn poll_video_task(api: &ApiClient, task_id: &str) -> Result<VideoTaskResponse> {
    let path = format!("videos/{task_id}");
    let started = Instant::now();
    loop {
        let resp = api.get_json(&path)?;
        if resp.status() == StatusCode::SERVICE_UNAVAILABLE {
            let wait = poll_interval(started.elapsed());
            log::debug!(
                "video task {task_id} poll: 503 service unavailable, retry in {}s",
                wait.as_secs()
            );
            thread::sleep(wait);
            continue;
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            logging::log_response(status.as_u16(), &text);
            bail!("poll video failed ({status}): {text}");
        }
        let task: VideoTaskResponse = resp.json().context("parse video task")?;
        match task.status.as_str() {
            "completed" => {
                log::debug!("video task {task_id} completed");
                return Ok(task);
            }
            "failed" => {
                log::debug!("video task {task_id} failed: {:?}", task.error);
                bail!("video task failed: {:?}", task.error);
            }
            status => {
                let wait = poll_interval(started.elapsed());
                log::debug!(
                    "video task {task_id} status={status}, next poll in {}s",
                    wait.as_secs()
                );
                thread::sleep(wait);
            }
        }
    }
}

fn poll_and_finalize(
    api: &ApiClient,
    task_id: &str,
    req: &VideoRequest,
    input_record: Option<serde_json::Value>,
) -> Result<GenerationResult> {
    let task = retry(api.config.max_retries, || poll_video_task(api, task_id))?;
    let remote = task.result_url().context("completed task missing video url")?;

    let save_local = req.save_local;
    let max_retries = req.max_retries.unwrap_or(api.config.max_retries);

    let (dims, ratio_label) = if req.task_id.is_some() && req.images.is_empty() {
        (video_dimensions(&req.ratio), req.ratio.label())
    } else if !req.images.is_empty() {
        let inputs_raw = parse_image_inputs(&req.images)?;
        let resolved: Vec<String> = inputs_raw
            .iter()
            .map(|s| api.db.resolve_reference(s))
            .collect::<Result<_>>()?;
        let classified: Vec<_> = resolved.iter().map(|s| classify_input(s)).collect();
        if classified.iter().all(|i| matches!(i, ImageInput::Url(_))) {
            let (dims, ratio, _) = video_frame_urls(&classified, &api.client)?;
            (dims, ratio.label())
        } else {
            let (_, ratio, dims) = prepare_video_frames(&classified, &api.client)?;
            (dims, ratio.label())
        }
    } else {
        (video_dimensions(&req.ratio), req.ratio.label())
    };

    let uri = if save_local {
        log::debug!("downloading video to local output dir");
        let output_dir = api.config.resolved_output_dir()?;
        let out_dir = req
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

    let asset = api
        .db
        .insert_asset("video", &remote, Some(&ratio_label), Some(&dims.size_string()))?;

    let mut result = GenerationResult {
        kind: "video".into(),
        ratio: ratio_label,
        size: dims.size_string(),
        uri,
        asset_uri: Some(asset.asset_uri),
        generation_id: None,
    };

    if let Some(input) = input_record {
        let output_record = serde_json::to_value(&result).context("serialize output")?;
        let gen_id = api
            .db
            .insert_generation("video", Some(&req.prompt), &input, &output_record, Some(&asset.id))?;
        result.generation_id = Some(gen_id);
    }

    log::debug!(
        "recorded asset={} generation_id={:?}",
        result.asset_uri.as_deref().unwrap_or("-"),
        result.generation_id
    );

    if !req.quiet {
        result.print(req.output_format)?;
    }
    Ok(result)
}

fn video_frame_urls(
    inputs: &[ImageInput],
    client: &reqwest::blocking::Client,
) -> Result<(crate::ratio::Dimensions, AspectRatio, Vec<String>)> {
    let urls: Vec<String> = inputs
        .iter()
        .map(|i| match i {
            ImageInput::Url(u) => Ok(u.clone()),
            _ => bail!("expected URL frame inputs"),
        })
        .collect::<Result<_>>()?;

    let mut dims = Vec::new();
    for input in inputs {
        let bytes = load_image_bytes(input, client)?;
        dims.push(image_dimensions_from_bytes(&bytes)?);
    }
    ensure_same_ratio(&dims)?;
    let ratio = AspectRatio::from_dimensions(dims[0].0, dims[0].1);
    let target = video_dimensions(&ratio);
    Ok((target, ratio, urls))
}

fn stage_frame_as_url(api: &ApiClient, data_uri: &str, dims: &crate::ratio::Dimensions) -> Result<String> {
    log::debug!("staging local frame via image API (data:image/jpeg;base64,...)");
    let body = ImageGenerationRequest {
        model: api.config.image_model.clone(),
        prompt: "Reproduce the input image exactly with no changes.".into(),
        size: dims.size_string(),
        extra_body: Some(ExtraBodyImage {
            image: Some(vec![data_uri.to_string()]),
            response_format: Some("url".into()),
            seed: None,
        }),
    };
    let resp = api.post_json("images/generations", &body)?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        logging::log_response(status.as_u16(), &text);
        bail!("staging frame for video failed ({status}): {text}");
    }
    let parsed: ImageGenerationResponse = resp.json().context("parse staging response")?;
    let item = parsed.data.first().context("empty staging response")?;
    let url = item.url.clone().context("staging response missing url")?;
    log::debug!("staged frame url: {url}");
    Ok(url)
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
}
