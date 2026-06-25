use std::thread;
use std::time::Duration;

use anyhow::Result;
use reqwest::StatusCode;

/// HTTP statuses that warrant a transient retry (429, 408, 503, 5xx).
pub fn is_retryable_http_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::SERVICE_UNAVAILABLE
        || status.is_server_error()
}

/// Exponential backoff with a small deterministic jitter (no extra deps).
pub fn backoff_delay(attempt: u32) -> Duration {
    let exp = attempt.saturating_sub(1).min(4);
    let base_ms = 100u64.saturating_mul(1u64 << exp);
    let jitter_ms = (attempt as u64 * 17) % 50;
    Duration::from_millis(base_ms + jitter_ms)
}

pub fn retry<T, F>(max_retries: u32, mut f: F) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    let mut attempt = 0u32;
    loop {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => {
                attempt += 1;
                if attempt > max_retries {
                    return Err(e);
                }
                let delay = backoff_delay(attempt);
                log::debug!("retry {attempt}/{max_retries} after error: {e:#}");
                thread::sleep(delay);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_includes_429_503_and_5xx() {
        assert!(is_retryable_http_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_http_status(StatusCode::REQUEST_TIMEOUT));
        assert!(is_retryable_http_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_retryable_http_status(StatusCode::INTERNAL_SERVER_ERROR));
    }

    #[test]
    fn not_retryable_401_or_422() {
        assert!(!is_retryable_http_status(StatusCode::UNAUTHORIZED));
        assert!(!is_retryable_http_status(StatusCode::UNPROCESSABLE_ENTITY));
    }

    #[test]
    fn backoff_grows_with_attempt() {
        assert!(backoff_delay(2) >= backoff_delay(1));
        assert!(backoff_delay(3) >= backoff_delay(2));
    }
}
