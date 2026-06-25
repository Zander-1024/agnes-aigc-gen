use anyhow::{Context, Result};
use reqwest::StatusCode;
use thiserror::Error;

use crate::logging;

#[derive(Debug, Error)]
pub enum AgnesApiError {
    #[error("authentication failed ({status}): {body}")]
    Auth { status: u16, body: String },

    #[error("rate limited ({status}): {body}")]
    RateLimited { status: u16, body: String },

    #[error("server error ({status}): {body}")]
    Server { status: u16, body: String },

    #[error("request failed ({status}): {body}")]
    Client { status: u16, body: String },

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("failed to parse response: {0}")]
    Parse(String),
}

impl AgnesApiError {
    pub fn from_http(status: StatusCode, body: impl Into<String>) -> Self {
        let body = body.into();
        let code = status.as_u16();
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Self::Auth { status: code, body },
            StatusCode::TOO_MANY_REQUESTS => Self::RateLimited { status: code, body },
            s if s.is_server_error() => Self::Server { status: code, body },
            _ => Self::Client { status: code, body },
        }
    }

    #[allow(dead_code)]
    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::Auth { status, .. }
            | Self::RateLimited { status, .. }
            | Self::Server { status, .. }
            | Self::Client { status, .. } => Some(*status),
            Self::Network(_) | Self::Parse(_) => None,
        }
    }

    #[allow(dead_code)]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::RateLimited { .. } | Self::Server { .. }) || matches!(self, Self::Network(_))
    }
}

/// Log and return a typed HTTP failure (for call sites that already consumed the body).
pub fn http_failure<T>(status: StatusCode, body: impl Into<String>, operation: impl Into<String>) -> Result<T> {
    let body = body.into();
    logging::log_response(status.as_u16(), &body);
    Err(AgnesApiError::from_http(status, body)).context(operation.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_auth_and_rate_limit() {
        let auth = AgnesApiError::from_http(StatusCode::UNAUTHORIZED, "bad key");
        assert!(matches!(auth, AgnesApiError::Auth { status: 401, .. }));
        assert!(!auth.is_retryable());

        let limited = AgnesApiError::from_http(StatusCode::TOO_MANY_REQUESTS, "slow down");
        assert!(matches!(limited, AgnesApiError::RateLimited { status: 429, .. }));
        assert!(limited.is_retryable());
    }

    #[test]
    fn maps_server_and_client() {
        let server = AgnesApiError::from_http(StatusCode::INTERNAL_SERVER_ERROR, "oops");
        assert!(matches!(server, AgnesApiError::Server { status: 500, .. }));
        assert!(server.is_retryable());

        let client = AgnesApiError::from_http(StatusCode::UNPROCESSABLE_ENTITY, "invalid");
        assert!(matches!(client, AgnesApiError::Client { status: 422, .. }));
        assert!(!client.is_retryable());
    }

    #[test]
    fn http_status_reports_code() {
        let err = AgnesApiError::from_http(StatusCode::FORBIDDEN, "nope");
        assert_eq!(err.http_status(), Some(403));
    }
}
