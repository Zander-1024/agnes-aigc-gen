use anyhow::Result;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::db::{Database, RequestRunInput};

pub fn body_hash(body: &Value) -> String {
    Sha256::digest(serde_json::to_vec(body).expect("JSON values serialize"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
    matches!(
        normalized.as_str(),
        "apikey"
            | "apikeyencrypted"
            | "authorization"
            | "password"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "secret"
            | "signature"
    )
}

/// Preserve existing history content while masking credential fields recursively.
pub fn redact_credentials(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        if sensitive_key(key) {
                            json!("[REDACTED]")
                        } else {
                            redact_credentials(value)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_credentials).collect()),
        _ => value.clone(),
    }
}

/// Keep parameter values, but summarize content and inline image bytes.
pub fn snapshot(body: &Value) -> Value {
    fn walk(value: &Value, key: &str) -> Value {
        if sensitive_key(key) {
            return json!("[REDACTED]");
        }
        if matches!(key, "prompt" | "messages") {
            return json!({"content_hash": body_hash(value), "bytes": value.to_string().len()});
        }
        match value {
            Value::String(text) if text.starts_with("data:") => json!({
                "inline_media": text.split(',').next().unwrap_or("data:"),
                "bytes": text.len(), "sha256": body_hash(value)
            }),
            Value::String(text) if text.starts_with("https://") || text.starts_with("http://") => {
                let mut url = match url::Url::parse(text) {
                    Ok(url) => url,
                    Err(_) => return value.clone(),
                };
                let _ = url.set_username("");
                let _ = url.set_password(None);
                url.set_query(None);
                url.set_fragment(None);
                json!(url.as_str())
            }
            Value::Object(map) => {
                Value::Object(map.iter().map(|(key, value)| (key.clone(), walk(value, key))).collect())
            }
            Value::Array(items) => Value::Array(items.iter().map(|value| walk(value, key)).collect()),
            _ => value.clone(),
        }
    }
    walk(body, "")
}

pub fn start_in(
    db: &Database,
    kind: &str,
    model: &str,
    endpoint: &str,
    config_hash: &str,
    layers: &[String],
    body: &Value,
) -> Result<i64> {
    let endpoint = snapshot(&json!(endpoint))
        .as_str()
        .unwrap_or("[invalid endpoint]")
        .to_string();
    db.start_request_run(&RequestRunInput {
        kind,
        model,
        endpoint: &endpoint,
        config_hash,
        layers,
        body: &snapshot(body),
        body_hash: &body_hash(body),
    })
}

/// Provider errors may echo credentials or request content; retain correlation only.
pub fn error_summary(error: &str) -> String {
    format!(
        "request error ({} bytes; details_sha256={})",
        error.len(),
        body_hash(&json!(error))
    )
}

pub struct RequestLog {
    db: Database,
    id: i64,
    finished: bool,
}

impl RequestLog {
    pub fn start(
        kind: &str,
        model: &str,
        endpoint: &str,
        config_hash: &str,
        layers: &[String],
        body: &Value,
    ) -> Result<Self> {
        let db = Database::open()?;
        let id = start_in(&db, kind, model, endpoint, config_hash, layers, body)?;
        Ok(Self { db, id, finished: false })
    }

    pub fn id(&self) -> i64 {
        self.id
    }

    pub fn context(&mut self, session_id: Option<&str>, turn: Option<u32>) -> Result<()> {
        self.db.link_request_context(self.id, session_id, turn)
    }

    pub fn attempt(&mut self) -> Result<()> {
        self.db.request_attempt(self.id)
    }

    pub fn response_status(&mut self, status: u16) -> Result<()> {
        self.db.finish_request_run(self.id, "streaming", Some(status), None)
    }

    pub fn success(&mut self, remote_id: Option<&str>) -> Result<()> {
        if let Some(remote_id) = remote_id {
            self.db.link_request_remote_id(self.id, remote_id)?;
        }
        self.db.finish_request_run(self.id, "succeeded", None, None)?;
        self.finished = true;
        Ok(())
    }

    pub fn failure(&mut self, error: &str, http_status: Option<u16>) -> Result<()> {
        self.db
            .finish_request_run(self.id, "failed", http_status, Some(&error_summary(error)))?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for RequestLog {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.db.finish_request_run(self.id, "interrupted", None, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_effective_parameters_and_attempts_without_content_or_credentials() {
        let db = Database::open_memory().unwrap();
        let body = json!({"temperature":0.4,"custom":{"keep":true,"token":"private"},"prompt":"private prompt","images":["data:image/jpeg;base64,AAAA"],"url":"https://user:password@example.com/a?signature=private"});
        let id = start_in(
            &db,
            "image",
            "model",
            "https://endpoint-user:endpoint-password@example.com/v1/images/generations?signature=secret",
            "cfg",
            &["image".into()],
            &body,
        )
        .unwrap();
        db.request_attempt(id).unwrap();
        db.request_attempt(id).unwrap();
        db.finish_request_run(id, "failed", Some(429), Some("rate limited"))
            .unwrap();
        let saved = db.get_request_run(id).unwrap();
        assert_eq!(saved["attempts"], 2);
        assert_eq!(saved["http_status"], 429);
        assert_eq!(saved["effective_params"]["custom"]["keep"], true);
        assert_eq!(saved["body_hash"], body_hash(&body));
        let text = saved.to_string();
        for private in
            ["private prompt", "AAAA", "password", "endpoint-user", "signature=private", "\"token\":\"private\""]
        {
            assert!(!text.contains(private));
        }
        assert_eq!(db.list_request_runs(1).unwrap().len(), 1);
    }

    #[test]
    fn provider_error_details_are_not_copied_to_history() {
        let error = "provider returned {\"token\":\"private-token\",\"prompt\":\"private-prompt\",\"image\":\"data:image/png;base64,AAAA\"}";
        let summary = error_summary(error);
        assert!(summary.contains("details_sha256="));
        for private in ["private-token", "private-prompt", "AAAA", "data:image"] {
            assert!(!summary.contains(private));
        }
    }

    #[test]
    fn text_record_tracks_context_and_final_outcome_with_an_immutable_snapshot() {
        let db = Database::open_memory().unwrap();
        let mut body = json!({"max_tokens":100,"vendor":{"enabled":true}});
        let id = start_in(
            &db,
            "text",
            "model",
            "https://example.com/v1/chat/completions",
            "cfg",
            &[],
            &body,
        )
        .unwrap();
        body["max_tokens"] = json!(200);
        let mut record = RequestLog { db, id, finished: false };
        record.context(Some("session-fixture"), Some(2)).unwrap();
        record.attempt().unwrap();
        record.response_status(201).unwrap();
        record.success(Some("provider-fixture")).unwrap();
        let saved = record.db.get_request_run(record.id()).unwrap();
        assert_eq!(saved["status"], "succeeded");
        assert_eq!(saved["http_status"], 201);
        assert_eq!(saved["session_id"], "session-fixture");
        assert_eq!(saved["turn"], 2);
        assert_eq!(saved["remote_id"], "provider-fixture");
        assert_eq!(saved["effective_params"]["max_tokens"], 100);
        assert!(record.finished);
    }
}
