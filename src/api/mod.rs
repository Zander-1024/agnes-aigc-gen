mod deliver;
pub(crate) mod endpoints;
mod error;
mod image;
pub mod types;
mod video;

pub use image::{ImageRequest, generate_image};
pub use types::VideoReference;
pub use video::{VideoRequest, generate_video, refresh_video_task, wait_video_task};

use std::cell::Cell;
use std::time::Duration;

use anyhow::Result;
use reqwest::blocking::{Client, Response};

use crate::config::{AppConfig, RequestKind};
use crate::db::Database;
use crate::logging;
use crate::retry::{is_retryable_http_status, retry};

pub struct ApiClient {
    pub client: Client,
    pub config: AppConfig,
    pub api_key: String,
    pub db: Database,
    pub last_request_id: Cell<Option<i64>>,
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
        Ok(Self { client, config, api_key, db, last_request_id: Cell::new(None) })
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

    #[cfg(test)]
    pub fn post_json(&self, path: &str, body: &impl serde::Serialize) -> Result<Response> {
        let body_value = serde_json::to_value(body)?;
        let kind = if path == "videos" {
            RequestKind::Video
        } else {
            RequestKind::Image
        };
        let model = body_value["model"].as_str().unwrap_or_default();
        let metadata =
            self.config
                .request_params
                .resolve(kind, model, &serde_json::json!({}), &serde_json::json!({}))?;
        self.post_json_with_config(path, body, &metadata.config_hash, &metadata.layers)
    }

    pub fn post_json_with_config(
        &self,
        path: &str,
        body: &impl serde::Serialize,
        config_hash: &str,
        layers: &[String],
    ) -> Result<Response> {
        let url = endpoints::v1_endpoint(&self.config.base_url, path)?;
        let body_value = serde_json::to_value(body)?;
        logging::log_request("POST", &url, Some(&body_value));
        let kind = if path == "videos" {
            RequestKind::Video
        } else {
            RequestKind::Image
        };
        let model = body_value["model"].as_str().unwrap_or_default();
        let id = crate::request_log::start_in(&self.db, kind.as_str(), model, &url, config_hash, layers, &body_value)?;
        self.last_request_id.set(Some(id));
        let mut http_status = None;
        let response = retry(self.config.max_retries, || {
            self.db.request_attempt(id)?;
            let resp = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body_value)
                .send()?;
            http_status = Some(resp.status().as_u16());
            if is_retryable_http_status(resp.status()) {
                anyhow::bail!("retryable status {}", resp.status());
            }
            Ok(resp)
        });
        let resp = match response {
            Ok(resp) => resp,
            Err(error) => {
                self.db.finish_request_run(
                    id,
                    "failed",
                    http_status,
                    Some(&crate::request_log::error_summary(&error.to_string())),
                )?;
                return Err(error);
            }
        };
        self.db.finish_request_run(
            id,
            if resp.status().is_success() {
                "received"
            } else {
                "failed"
            },
            http_status,
            None,
        )?;
        log::debug!("response status={}", resp.status());
        Ok(resp)
    }

    pub fn get_json_url(&self, url: &str) -> Result<Response> {
        logging::log_request("GET", url, None);
        let resp = retry(self.config.max_retries, || {
            let resp = self.client.get(url).bearer_auth(&self.api_key).send()?;
            if is_retryable_http_status(resp.status()) {
                anyhow::bail!("retryable status {}", resp.status());
            }
            Ok(resp)
        })?;
        log::debug!("response status={}", resp.status());
        Ok(resp)
    }

    /// Map a non-success HTTP response to a typed API error (logs body).
    pub fn ensure_success(resp: Response, operation: impl Into<String>) -> Result<Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().unwrap_or_default();
        error::http_failure(status, body, operation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn request_history_records_http_failure_and_retries_the_same_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            for status in [503, 400] {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 1024];
                let end = loop {
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        break end + 4;
                    }
                    let size = stream.read(&mut buffer).unwrap();
                    assert!(size > 0);
                    bytes.extend_from_slice(&buffer[..size]);
                };
                let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                while bytes.len() < end + length {
                    let size = stream.read(&mut buffer).unwrap();
                    assert!(size > 0);
                    bytes.extend_from_slice(&buffer[..size]);
                }
                bodies.push(bytes[end..end + length].to_vec());
                write!(
                    stream,
                    "HTTP/1.1 {status} Fixture\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                )
                .unwrap();
            }
            bodies
        });
        let api = ApiClient {
            client: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap(),
            config: AppConfig { base_url: format!("http://{address}"), max_retries: 2, ..AppConfig::default() },
            api_key: "fixture-key".into(),
            db: Database::open_memory().unwrap(),
            last_request_id: Cell::new(None),
        };
        let body = serde_json::json!({"model":"fixture","vendor":{"retained":true}});
        let response = api.post_json("images/generations", &body).unwrap();
        assert_eq!(response.status().as_u16(), 400);
        let bodies = server.join().unwrap();
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0], bodies[1]);
        let saved = api.db.get_request_run(api.last_request_id.get().unwrap()).unwrap();
        assert_eq!(saved["status"], "failed");
        assert_eq!(saved["http_status"], 400);
        assert_eq!(saved["attempts"], 2);
        assert_eq!(saved["effective_params"]["vendor"]["retained"], true);
        assert_eq!(api.db.list_request_runs(20).unwrap().len(), 1);
    }
}
