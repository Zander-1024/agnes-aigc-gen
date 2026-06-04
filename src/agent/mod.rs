use anyhow::Result;

use crate::output::GenerationResult;

pub mod agnes_stream;
pub mod approval;
pub mod chat;
pub mod context;
pub mod runner;
pub mod session;
pub mod tools;

pub trait TaskRouter {
    fn route_image(&self, prompt: &str, ratio: &str, inputs: &[String]) -> Result<GenerationResult>;
    fn route_video(&self, prompt: &str, ratio: &str, duration: f64, images: &[String]) -> Result<GenerationResult>;
}

pub struct LocalTaskRouter;

impl TaskRouter for LocalTaskRouter {
    fn route_image(&self, prompt: &str, ratio: &str, inputs: &[String]) -> Result<GenerationResult> {
        let cfg = crate::config::AppConfig::load()?;
        let api = crate::api::ApiClient::from_config(cfg)?;
        crate::api::generate_image(
            &api,
            crate::api::ImageRequest {
                prompt: prompt.to_string(),
                ratio: crate::ratio::AspectRatio::parse(ratio)?,
                inputs: inputs.to_vec(),
                seed: None,
                output_dir: None,
                save_local: false,
                max_retries: None,
                output_format: crate::output::OutputFormat::Json,
                quiet: true,
            },
        )
    }

    fn route_video(&self, prompt: &str, ratio: &str, duration: f64, images: &[String]) -> Result<GenerationResult> {
        let cfg = crate::config::AppConfig::load()?;
        let api = crate::api::ApiClient::from_config(cfg)?;
        crate::api::generate_video(
            &api,
            crate::api::VideoRequest {
                prompt: prompt.to_string(),
                negative_prompt: None,
                seed: None,
                ratio: crate::ratio::AspectRatio::parse(ratio)?,
                duration,
                frame_rate: 24,
                images: images.to_vec(),
                task_id: None,
                async_mode: false,
                output_dir: None,
                save_local: false,
                max_retries: None,
                output_format: crate::output::OutputFormat::Json,
                quiet: true,
            },
        )
    }
}

#[cfg(feature = "agent")]
pub mod pi;
