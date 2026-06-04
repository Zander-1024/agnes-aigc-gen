mod image;
mod types;
mod video;

pub use image::{ImageRequest, generate_image};
pub use video::{VideoRequest, generate_video};

use std::time::Duration;

use anyhow::Result;
use log;
use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};

use crate::config::AppConfig;
use crate::db::Database;
use crate::logging;
use crate::output::retry;

pub struct ApiClient {
    pub client: Client,
    pub config: AppConfig,
    pub api_key: String,
    pub db: Database,
}

impl ApiClient {
    pub fn from_config(config: AppConfig) -> Result<Self> {
        let api_key = config.api_key()?;
        let client = Client::builder().timeout(Duration::from_secs(300)).build()?;
        let db = Database::open()?;
        log::debug!("base_url={}", config.base_url);
        log::debug!("image_model={}", config.image_model);
        log::debug!("video_model={}", config.video_model);
        log::debug!("output_dir={}", config.output_dir);
        log::debug!("save_local={}", config.save_local);
        log::debug!("max_retries={}", config.max_retries);
        if let Ok(path) = Database::db_path() {
            log::debug!("db={}", path.display());
        }
        log::debug!("api_key_set={}", !api_key.is_empty());
        Ok(Self { client, config, api_key, db })
    }

    pub fn with_overrides(
        config: AppConfig,
        output_dir: Option<String>,
        save_local: Option<bool>,
        max_retries: Option<u32>,
    ) -> Result<Self> {
        let mut cfg = config;
        if let Some(dir) = output_dir {
            cfg.output_dir = dir;
        }
        if let Some(save) = save_local {
            cfg.save_local = save;
        }
        if let Some(retries) = max_retries {
            cfg.max_retries = retries;
        }
        Self::from_config(cfg)
    }

    pub fn post_json(&self, path: &str, body: &impl serde::Serialize) -> Result<Response> {
        let url = format!(
            "{}/{}",
            self.config.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        let body_value = serde_json::to_value(body).ok();
        logging::log_request("POST", &url, body_value.as_ref());
        let resp = retry(self.config.max_retries, || {
            let resp = self.client.post(&url).bearer_auth(&self.api_key).json(body).send()?;
            if should_retry_status(resp.status()) {
                anyhow::bail!("retryable status {}", resp.status());
            }
            Ok(resp)
        })?;
        log::debug!("response status={}", resp.status());
        Ok(resp)
    }

    pub fn get_json(&self, path: &str) -> Result<Response> {
        let url = format!(
            "{}/{}",
            self.config.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        logging::log_request("GET", &url, None);
        let resp = retry(self.config.max_retries, || {
            let resp = self.client.get(&url).bearer_auth(&self.api_key).send()?;
            if should_retry_status(resp.status()) {
                anyhow::bail!("retryable status {}", resp.status());
            }
            Ok(resp)
        })?;
        log::debug!("response status={}", resp.status());
        Ok(resp)
    }
}

fn should_retry_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE || status.is_server_error()
}
