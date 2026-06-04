use anyhow::Result;
use clap::Args;
use log;

use crate::api::{ApiClient, VideoRequest, generate_video};
use crate::config::AppConfig;
use crate::output::OutputFormat;
use crate::ratio::{AspectRatio, max_video_duration, resolve_video_timing, validate_frame_rate};

#[derive(Args)]
pub struct VideoArgs {
    /// Text prompt describing the video
    #[arg(short = 'p', long = "prompt")]
    pub prompt: Option<String>,

    /// Negative prompt (undesired content)
    #[arg(long = "negative-prompt", visible_alias = "np")]
    pub negative_prompt: Option<String>,

    /// Fixed seed for reproducible generation (0–999). Omitted = not sent to API.
    #[arg(short = 's', long = "seed")]
    pub seed: Option<u32>,

    /// Aspect ratio when no input images (e.g. 16:9)
    #[arg(short = 'r', long = "ratio", default_value = "16:9")]
    pub ratio: String,

    /// Target duration in seconds (max depends on frame rate; 18s at 24 fps)
    #[arg(short = 'd', long = "duration", default_value_t = 5.0)]
    pub duration: f64,

    /// Frame rate (1–60, default 24). Max duration = floor(441 / fps) seconds.
    #[arg(short = 'f', long = "frame-rate", default_value_t = 24)]
    pub frame_rate: u32,

    /// Input image URL(s): HTTPS URL or asset:// only (repeatable or comma-separated)
    #[arg(short = 'i', long = "image")]
    pub images: Vec<String>,

    /// Poll an existing task by ID
    #[arg(long = "task-id")]
    pub task_id: Option<String>,

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

pub fn run(args: VideoArgs) -> Result<()> {
    validate_frame_rate(args.frame_rate)?;

    let cfg = AppConfig::load()?;
    let ratio = AspectRatio::parse(&args.ratio)?;
    let output_format = match args.output_format.to_lowercase().as_str() {
        "json" => OutputFormat::Json,
        "plain" => OutputFormat::Plain,
        other => anyhow::bail!("unknown output format: {other}"),
    };

    let prompt = match (&args.prompt, &args.task_id) {
        (Some(p), _) => p.clone(),
        (None, Some(_)) => String::new(),
        (None, None) => anyhow::bail!("--prompt is required unless --task-id is provided"),
    };

    if args.task_id.is_none() {
        let max_dur = max_video_duration(args.frame_rate)?;
        let (num_frames, actual_duration) = resolve_video_timing(args.duration, args.frame_rate)?;
        log::debug!(
            "video timing: requested={}s max={max_dur}s frames={num_frames} actual={actual_duration:.3}s fps={}",
            args.duration,
            args.frame_rate
        );
    }

    let api = ApiClient::with_overrides(cfg, args.output_dir.clone(), None, args.retries)?;

    generate_video(
        &api,
        VideoRequest {
            prompt,
            negative_prompt: args.negative_prompt,
            seed: args.seed,
            ratio,
            duration: args.duration,
            frame_rate: args.frame_rate,
            images: args.images,
            task_id: args.task_id,
            output_dir: args.output_dir,
            save_local: args.save,
            max_retries: args.retries,
            output_format,
            quiet: false,
        },
    )?;
    Ok(())
}
