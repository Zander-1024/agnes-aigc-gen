use anyhow::{Context, Result, ensure};
use url::Url;

/// Build a gateway endpoint, accepting a base with or without a final `/v1`.
pub(crate) fn gateway_endpoint(base_url: &str, path: &str) -> Result<Url> {
    let mut url = Url::parse(base_url.trim()).context("parse base_url")?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "base_url must be an HTTP(S) gateway URL"
    );
    let base_path = url.path().trim_end_matches('/');
    let root_path = base_path.strip_suffix("/v1").unwrap_or(base_path);
    let endpoint_path = format!("{}/{}", root_path, path.trim_start_matches('/'));
    url.set_path(&endpoint_path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

/// Build an OpenAI-compatible endpoint with exactly one API version suffix.
pub(crate) fn v1_endpoint(base_url: &str, path: &str) -> Result<String> {
    let path = path.trim_start_matches('/');
    let versioned_path = if path.is_empty() {
        "v1".to_string()
    } else {
        format!("v1/{path}")
    };
    gateway_endpoint(base_url, &versioned_path).map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_and_versioned_bases_resolve_to_the_same_api_endpoints() {
        for base in [
            "https://example.com",
            "https://example.com/",
            "https://example.com/v1",
            "https://example.com/v1/",
            " https://example.com/v1/// ",
        ] {
            for path in ["chat/completions", "images/generations", "videos"] {
                assert_eq!(
                    v1_endpoint(base, path).unwrap(),
                    format!("https://example.com/v1/{path}")
                );
            }
            assert_eq!(v1_endpoint(base, "").unwrap(), "https://example.com/v1");
            assert_eq!(
                gateway_endpoint(base, "agnesapi").unwrap().as_str(),
                "https://example.com/agnesapi"
            );
        }
    }

    #[test]
    fn custom_gateway_prefix_and_port_are_preserved() {
        for base in ["http://localhost:8080/proxy", "http://localhost:8080/proxy/v1/"] {
            assert_eq!(
                v1_endpoint(base, "/videos").unwrap(),
                "http://localhost:8080/proxy/v1/videos"
            );
            assert_eq!(
                gateway_endpoint(base, "/agnesapi").unwrap().as_str(),
                "http://localhost:8080/proxy/agnesapi"
            );
        }
    }

    #[test]
    fn endpoints_do_not_inherit_base_query_or_fragment() {
        let base = "https://example.com/v1/?old=query#fragment";
        assert_eq!(
            v1_endpoint(base, "images/generations").unwrap(),
            "https://example.com/v1/images/generations"
        );
        assert_eq!(
            gateway_endpoint(base, "agnesapi").unwrap().as_str(),
            "https://example.com/agnesapi"
        );
    }

    #[test]
    fn invalid_gateway_urls_fail_before_a_request() {
        for base in ["not a URL", "file:///tmp/gateway", "ftp://example.com"] {
            assert!(v1_endpoint(base, "videos").is_err());
            assert!(gateway_endpoint(base, "agnesapi").is_err());
        }
    }
}
