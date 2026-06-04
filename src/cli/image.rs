use std::sync::mpsc;
use std::thread;

use anyhow::Result;
use clap::Args;

use crate::api::{ApiClient, ImageRequest, generate_image};
use crate::config::AppConfig;
use crate::output::{ImageBatchItem, MAX_IMAGE_BATCH_COUNT, OutputFormat, print_batch_results};
use crate::ratio::AspectRatio;

#[derive(Args)]
pub struct ImageArgs {
    /// Text prompt for generation or editing
    #[arg(short = 'p', long = "prompt")]
    pub prompt: String,

    /// Aspect ratio (e.g. 16:9, 1:1, 4:3)
    #[arg(short = 'r', long = "ratio", default_value = "1:1")]
    pub ratio: String,

    /// Number of images to generate (concurrent API calls, max 4)
    #[arg(short = 'n', long = "count", default_value_t = 1)]
    pub count: u32,

    /// Fixed seed for generation (0–999). Omit for random perturbation in 0–999 per call. Not compatible with --count > 1.
    #[arg(short = 's', long = "seed")]
    pub seed: Option<u32>,

    /// Input image(s): local path, URL, asset://, base64, or data URI
    #[arg(short = 'i', long = "input")]
    pub inputs: Vec<String>,

    /// Override output directory
    #[arg(long = "output-dir")]
    pub output_dir: Option<String>,

    /// Download result to output-dir (off by default; uri uses remote URL)
    #[arg(long = "save")]
    pub save: bool,

    /// Max retry attempts for API and download
    #[arg(long = "retries")]
    pub retries: Option<u32>,

    /// Output format: json or plain
    #[arg(long = "output-format", default_value = "json")]
    pub output_format: String,
}

pub fn run(args: ImageArgs) -> Result<()> {
    anyhow::ensure!(
        (1..=MAX_IMAGE_BATCH_COUNT).contains(&args.count),
        "count must be 1–{MAX_IMAGE_BATCH_COUNT}"
    );
    anyhow::ensure!(
        args.count == 1 || args.seed.is_none(),
        "--seed / -s cannot be used with --count / -n > 1; omit seed for batch generation"
    );

    let cfg = AppConfig::load()?;
    let ratio = AspectRatio::parse(&args.ratio)?;
    let output_format = parse_output_format(&args.output_format)?;

    if args.count == 1 {
        let api = ApiClient::with_overrides(cfg, args.output_dir.clone(), None, args.retries)?;
        generate_image(
            &api,
            ImageRequest {
                prompt: args.prompt.clone(),
                ratio,
                inputs: args.inputs.clone(),
                seed: args.seed,
                output_dir: args.output_dir.clone(),
                save_local: args.save,
                max_retries: args.retries,
                output_format,
                quiet: false,
            },
        )?;
        return Ok(());
    }

    log::debug!("generating {} images concurrently", args.count);

    let (tx, rx) = mpsc::channel();
    thread::scope(|scope| {
        for _ in 0..args.count {
            let tx = tx.clone();
            let cfg = cfg.clone();
            let prompt = args.prompt.clone();
            let ratio = ratio.clone();
            let inputs = args.inputs.clone();
            let output_dir = args.output_dir.clone();
            let seed = args.seed;
            let save = args.save;
            let retries = args.retries;

            scope.spawn(move || {
                let item = match ApiClient::with_overrides(cfg, output_dir.clone(), None, retries) {
                    Ok(api) => match generate_image(
                        &api,
                        ImageRequest {
                            prompt,
                            ratio,
                            inputs,
                            seed,
                            output_dir,
                            save_local: save,
                            max_retries: retries,
                            output_format,
                            quiet: true,
                        },
                    ) {
                        Ok(result) => ImageBatchItem::from_result(result),
                        Err(err) => {
                            log::debug!("batch item failed: {err:#}");
                            ImageBatchItem::failure(format!("{err:#}"))
                        }
                    },
                    Err(err) => ImageBatchItem::failure(format!("{err:#}")),
                };
                let _ = tx.send(item);
            });
        }
    });

    let items: Vec<_> = rx.iter().collect();
    let any_success = items.iter().any(ImageBatchItem::is_success);

    print_batch_results(&items, output_format)?;

    if !any_success {
        anyhow::bail!("all generations failed");
    }

    Ok(())
}

fn parse_output_format(s: &str) -> Result<OutputFormat> {
    match s.to_lowercase().as_str() {
        "json" => Ok(OutputFormat::Json),
        "plain" => Ok(OutputFormat::Plain),
        other => anyhow::bail!("unknown output format: {other}"),
    }
}
