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
    log::debug!("response status={status} body={}", truncate_body(body, 2000));
}

fn truncate_body(body: &str, max: usize) -> String {
    if body.len() <= max {
        body.to_string()
    } else {
        format!("{}... ({} bytes total)", &body[..max], body.len())
    }
}

fn summarize_str(s: &str) -> String {
    if s.starts_with("data:") {
        let header = s.split(',').next().unwrap_or("data:");
        return format!("{header},... ({len} bytes)", len = s.len());
    }
    if s.len() > 120 {
        format!("{}... ({len} chars)", &s[..80], len = s.len())
    } else {
        s.to_string()
    }
}

fn redact_json(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(summarize_str(s)),
        Value::Array(items) => Value::Array(items.iter().map(redact_json).collect()),
        Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), redact_json(v))).collect()),
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
}
