use std::sync::mpsc;

use anyhow::Result;
use tokio::sync::mpsc as async_mpsc;

use crate::api::{ApiClient, ImageRequest, VideoRequest, generate_image, generate_video};
use crate::config::AppConfig;
use crate::output::{GenerationResult, OutputFormat};
use crate::ratio::AspectRatio;

pub enum JobRequest {
    Image(ImageJobParams),
    Video(VideoJobParams),
}

pub struct ImageJobParams {
    pub prompt: String,
    pub ratio: AspectRatio,
    pub inputs: Vec<String>,
    pub count: u32,
    pub seed: Option<u32>,
    pub output_dir: Option<String>,
    pub save_local: bool,
}

pub struct VideoJobParams {
    pub prompt: String,
    pub negative_prompt: Option<String>,
    pub ratio: AspectRatio,
    pub duration: f64,
    pub frame_rate: u32,
    pub images: Vec<String>,
    pub seed: Option<u32>,
    pub output_dir: Option<String>,
    pub save_local: bool,
    pub async_mode: bool,
}

pub enum JobEvent {
    ImageDone {
        results: Vec<GenerationResult>,
        error: Option<String>,
    },
    VideoSubmitted {
        local_id: i64,
        error: Option<String>,
    },
    VideoDone {
        results: Vec<GenerationResult>,
        error: Option<String>,
    },
}

pub struct JobHandle {
    pub rx: async_mpsc::UnboundedReceiver<JobEvent>,
}

pub fn spawn_job(request: JobRequest) -> JobHandle {
    let (tx, rx) = async_mpsc::unbounded_channel();
    let is_image = matches!(request, JobRequest::Image(_));
    let video_async = matches!(&request, JobRequest::Video(p) if p.async_mode);
    std::thread::spawn(move || {
        let result = match request {
            JobRequest::Image(params) => run_image_job(params),
            JobRequest::Video(params) => run_video_job(params),
        };
        match result {
            Ok(event) => {
                let _ = tx.send(event);
            }
            Err(err) => {
                let event = if is_image {
                    JobEvent::ImageDone { results: Vec::new(), error: Some(format!("{err:#}")) }
                } else if video_async {
                    JobEvent::VideoSubmitted { local_id: 0, error: Some(format!("{err:#}")) }
                } else {
                    JobEvent::VideoDone { results: Vec::new(), error: Some(format!("{err:#}")) }
                };
                let _ = tx.send(event);
            }
        }
    });
    JobHandle { rx }
}

fn run_image_job(params: ImageJobParams) -> Result<JobEvent> {
    let cfg = AppConfig::load()?;
    if params.count == 1 {
        let api = ApiClient::with_overrides(cfg, params.output_dir.clone(), None, None)?;
        let result = generate_image(
            &api,
            ImageRequest {
                prompt: params.prompt,
                ratio: params.ratio,
                inputs: params.inputs,
                seed: params.seed,
                output_dir: params.output_dir,
                save_local: params.save_local,
                max_retries: None,
                output_format: OutputFormat::Json,
                quiet: true,
            },
        )?;
        return Ok(JobEvent::ImageDone { results: vec![result], error: None });
    }

    let (tx, rx) = mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..params.count {
            let tx = tx.clone();
            let cfg = cfg.clone();
            let prompt = params.prompt.clone();
            let ratio = params.ratio.clone();
            let inputs = params.inputs.clone();
            let output_dir = params.output_dir.clone();
            scope.spawn(move || {
                let item = match ApiClient::with_overrides(cfg, output_dir.clone(), None, None) {
                    Ok(api) => generate_image(
                        &api,
                        ImageRequest {
                            prompt,
                            ratio,
                            inputs,
                            seed: None,
                            output_dir,
                            save_local: params.save_local,
                            max_retries: None,
                            output_format: OutputFormat::Json,
                            quiet: true,
                        },
                    ),
                    Err(err) => Err(err),
                };
                let _ = tx.send(item);
            });
        }
    });
    drop(tx);
    let mut results = Vec::new();
    let mut error = None;
    for item in rx {
        match item {
            Ok(result) => results.push(result),
            Err(err) => error = Some(format!("{err:#}")),
        }
    }
    Ok(JobEvent::ImageDone { results, error })
}

fn run_video_job(params: VideoJobParams) -> Result<JobEvent> {
    let cfg = AppConfig::load()?;
    let api = ApiClient::with_overrides(cfg, params.output_dir.clone(), None, None)?;
    let result = generate_video(
        &api,
        VideoRequest {
            prompt: params.prompt,
            negative_prompt: params.negative_prompt,
            seed: params.seed,
            ratio: params.ratio,
            duration: params.duration,
            frame_rate: params.frame_rate,
            images: params.images,
            task_id: None,
            async_mode: params.async_mode,
            output_dir: params.output_dir,
            save_local: params.save_local,
            max_retries: None,
            output_format: OutputFormat::Json,
            quiet: true,
        },
    )?;
    if params.async_mode {
        let local_id = result.generation_id.unwrap_or(0);
        Ok(JobEvent::VideoSubmitted {
            local_id,
            error: if local_id == 0 {
                Some("missing local task id".into())
            } else {
                None
            },
        })
    } else {
        Ok(JobEvent::VideoDone { results: vec![result], error: None })
    }
}
