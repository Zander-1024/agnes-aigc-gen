use anyhow::{Context, Result};
use clap::Args;

use crate::api::{ApiClient, VideoReference, VideoRequest, generate_video};
use crate::cli::task::resolve_task_ref;
use crate::cli::util::parse_output_format;
use crate::config::AppConfig;
use crate::ratio::{AspectRatio, validate_video_duration, video_dimensions_for_size};

#[derive(Args)]
pub struct VideoArgs {
    /// Text prompt describing the video
    #[arg(short = 'p', long = "prompt")]
    pub prompt: Option<String>,

    /// Agnes Video 2.5 model (overrides configured video model)
    #[arg(long, value_parser = ["agnes-video-2.5", "agnes-video-2.5-flash"])]
    pub model: Option<String>,

    /// Generation mode (default: infer from supplied media)
    #[arg(long, value_parser = ["text", "keyframe", "reference"])]
    pub mode: Option<String>,

    /// Output resolution tier (Flash supports 720P only)
    #[arg(long, default_value = "720P", value_parser = ["720P", "1080P", "1K", "2K"])]
    pub size: String,

    /// Fixed seed for reproducible generation. Omitted = not sent to API.
    #[arg(short = 's', long = "seed")]
    pub seed: Option<u32>,

    /// Output aspect ratio (e.g. 16:9)
    #[arg(short = 'r', long = "ratio", default_value = "16:9")]
    pub ratio: String,

    /// Target duration in whole seconds (4–12)
    #[arg(short = 'd', long = "duration", default_value_t = 5.0)]
    pub duration: f64,

    /// Input images: HTTPS URL, asset://, local path, Base64, or Data URI (repeatable or comma-separated)
    #[arg(short = 'i', long = "image")]
    pub images: Vec<String>,

    /// First frame image: HTTPS URL, asset://, local path, Base64, or Data URI
    #[arg(long)]
    pub first_frame: Option<String>,

    /// Last frame image: HTTPS URL, asset://, local path, Base64, or Data URI
    #[arg(long)]
    pub last_frame: Option<String>,

    /// Reference audio HTTPS URL (repeatable)
    #[arg(long = "audio")]
    pub audios: Vec<String>,

    /// Reference video HTTPS URL or JSON object with url/start_seconds/require_audio (repeatable; standard model only)
    #[arg(long = "video", value_parser = parse_video_reference)]
    pub videos: Vec<VideoReference>,

    /// Poll an existing task by ID (blocks until complete)
    #[arg(long = "task-id")]
    pub task_id: Option<String>,

    /// Submit video task and return immediately (use `task show` / `task wait` to follow up)
    #[arg(long = "async")]
    pub async_mode: bool,

    #[arg(long = "output-dir")]
    pub output_dir: Option<String>,

    /// Download result to output-dir (off by default; uri uses remote URL)
    #[arg(long = "save")]
    pub save: bool,

    #[arg(long = "retries")]
    pub retries: Option<u32>,

    #[arg(long = "output-format", default_value = "json")]
    pub output_format: String,
}

fn parse_video_reference(raw: &str) -> Result<VideoReference> {
    if raw.trim_start().starts_with('{') {
        serde_json::from_str(raw).context("video reference must contain url and optional start_seconds/require_audio")
    } else {
        Ok(VideoReference { url: raw.to_string(), start_seconds: None, require_audio: false })
    }
}

pub fn run(args: VideoArgs) -> Result<()> {
    let mut cfg = AppConfig::load()?;
    if let Some(model) = &args.model {
        cfg.video_model = model.clone();
    }
    let ratio = AspectRatio::parse(&args.ratio)?;
    let output_format = parse_output_format(&args.output_format)?;

    let prompt = match (&args.prompt, &args.task_id) {
        (Some(p), _) => p.clone(),
        (None, Some(_)) => String::new(),
        (None, None) => anyhow::bail!("--prompt is required unless --task-id is provided"),
    };

    if args.task_id.is_none() {
        validate_video_duration(args.duration)?;
        video_dimensions_for_size(&ratio, &args.size, cfg.video_model == "agnes-video-2.5-flash")?;
    }

    let api = ApiClient::with_overrides(cfg, args.output_dir.clone(), None, args.retries)?;
    let async_mode = args.async_mode && args.task_id.is_none();
    let task_id = match args.task_id {
        Some(ref tid) => Some(resolve_task_ref(tid)?),
        None => None,
    };

    generate_video(
        &api,
        VideoRequest {
            prompt,
            model: args.model,
            mode: args.mode,
            size: args.size,
            first_frame: args.first_frame,
            last_frame: args.last_frame,
            audios: args.audios,
            videos: args.videos,
            seed: args.seed,
            ratio,
            duration: args.duration,
            images: args.images,
            task_id,
            async_mode,
            output_dir: args.output_dir,
            save_local: args.save,
            max_retries: args.retries,
            output_format,
            quiet: false,
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_reference_accepts_url_and_structured_options() {
        let video = parse_video_reference("https://example.com/input.mp4").unwrap();
        assert_eq!(video.url, "https://example.com/input.mp4");
        assert!(!video.require_audio);
        let video = parse_video_reference(
            r#"{"url":"https://example.com/input.mp4","start_seconds":2.5,"require_audio":true}"#,
        )
        .unwrap();
        assert_eq!(video.start_seconds, Some(2.5));
        assert!(video.require_audio);
    }
}
