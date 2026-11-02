use std::io::Write;

use env_logger::Builder;
use log::LevelFilter;
use serde_json::Value;

pub fn init(verbose: bool) {
    let level = if verbose { LevelFilter::Debug } else { LevelFilter::Off };
    Builder::new()
        .filter_level(level)
        .format(|buf, record| writeln!(buf, "[{}] {}", record.level(), record.args()))
        .init();
}

pub fn log_response(status: u16, body: &str) {
    log::debug!("response status={status} body={}", response_summary(body));
}

fn response_summary(body: &str) -> String {
    let summary = match serde_json::from_str::<Value>(body) {
        Ok(value) => summarize_json(&value),
        Err(_) => body.to_string(),
    };
    truncate_body(&summary, 4096)
}

fn prefix_at_boundary(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn truncate_body(body: &str, max: usize) -> String {
    if body.len() <= max {
        body.to_string()
    } else {
        format!("{}... ({} bytes total)", prefix_at_boundary(body, max), body.len())
    }
}

fn summarize_str(s: &str) -> String {
    if s.starts_with("data:") {
        let header = s.split(',').next().unwrap_or("data:");
        return format!("{header},... ({len} bytes)", len = s.len());
    }
    if let Ok(mut url) = url::Url::parse(s)
        && matches!(url.scheme(), "http" | "https")
    {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        let has_query = url.query().is_some();
        url.set_query(None);
        url.set_fragment(None);
        let mut summary = truncate_body(url.as_str(), 1000);
        if has_query {
            summary.push_str("?[REDACTED]");
        }
        return summary;
    }
    if s.len() > 120 {
        format!(
            "{}... ({len} chars)",
            prefix_at_boundary(s, 80),
            len = s.chars().count()
        )
    } else {
        s.to_string()
    }
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
            | "b64json"
    )
}

fn redact_json(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(summarize_str(s)),
        Value::Array(items) => Value::Array(items.iter().map(redact_json).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let value = if sensitive_key(k) {
                        Value::String("[REDACTED]".to_string())
                    } else {
                        redact_json(v)
                    };
                    (k.clone(), value)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

pub fn summarize_json(value: &Value) -> String {
    serde_json::to_string(&redact_json(value)).unwrap_or_else(|_| value.to_string())
}

pub fn log_request(method: &str, url: &str, body: Option<&Value>) {
    log::debug!("{method} {url}");
    if let Some(body) = body {
        log::debug!("request body: {}", summarize_json(body));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_data_uri() {
        let v = json!({ "image": ["data:image/jpeg;base64,AAAA"] });
        let s = summarize_json(&v);
        assert!(s.contains("data:image/jpeg;base64,..."));
        assert!(!s.contains("AAAA"));
    }

    #[test]
    fn truncates_long_strings() {
        let long = "x".repeat(200);
        let v = json!({ "prompt": long });
        let s = summarize_json(&v);
        assert!(s.contains("200 chars"));
    }

    #[test]
    fn truncates_multibyte_text_at_character_boundaries() {
        let body = "海".repeat(1000);
        let summary = truncate_body(&body, 2000);
        assert!(summary.ends_with("... (3000 bytes total)"));
        let summary = summarize_json(&json!({"prompt": "海".repeat(100)}));
        assert!(summary.contains("100 chars"));
    }

    #[test]
    fn response_summary_keeps_result_urls_and_redacts_credentials() {
        let video_url = format!("https://storage.example.com/videos/{}.mp4", "v".repeat(100));
        let body = json!({
            "status": "completed",
            "metadata": {"url": video_url},
            "api_key": "key-that-must-not-be-logged",
            "authorization": "Bearer secret-token",
            "image": "data:image/png;base64,SECRET_IMAGE_BYTES",
            "signed_url": "https://user:password@example.com/video.mp4?X-Goog-Signature=secret-signature&expires=123"
        });
        let summary = response_summary(&body.to_string());
        assert!(summary.contains(&video_url));
        assert!(summary.contains("completed"));
        assert!(summary.contains("https://example.com/video.mp4"));
        for secret in
            ["key-that-must-not-be-logged", "secret-token", "SECRET_IMAGE_BYTES", "password", "secret-signature"]
        {
            assert!(!summary.contains(secret), "response log leaked {secret}");
        }
    }
}
