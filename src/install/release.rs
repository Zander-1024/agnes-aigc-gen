use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;

use super::platform::Platform;

pub const DEFAULT_REPO: &str = "Zander-1024/agnes-aigc-gen";
const BIN_NAME: &str = "agnes-aigc-gen";
const USER_AGENT: &str = concat!("agnes-aigc-gen/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Deserialize)]
struct LatestRelease {
    tag_name: String,
}

pub fn fetch_latest_version(repo: &str) -> Result<String> {
    let api_url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let latest_url = format!("https://github.com/{repo}/releases/latest");
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()?;
    fetch_latest_version_from(&client, &api_url, &latest_url)
}

fn fetch_latest_version_from(client: &Client, api_url: &str, latest_url: &str) -> Result<String> {
    match fetch_latest_from_api(client, api_url) {
        Ok(version) => Ok(version),
        Err(api_error) => fetch_latest_from_release_page(client, latest_url).map_err(|page_error| {
            anyhow::anyhow!(
                "Could not determine latest GitHub release. API lookup: {api_error:#}; release page lookup: {page_error:#}"
            )
        }),
    }
}

fn fetch_latest_from_api(client: &Client, api_url: &str) -> Result<String> {
    let response = client
        .get(api_url)
        .send()
        .context("fetch latest GitHub release")?
        .error_for_status()
        .context("GitHub releases/latest returned error")?;
    let release: LatestRelease = response.json().context("parse latest release JSON")?;
    validated_release_version(&release.tag_name)
}

fn fetch_latest_from_release_page(client: &Client, latest_url: &str) -> Result<String> {
    let latest_url = reqwest::Url::parse(latest_url).context("parse GitHub latest release URL")?;
    let response = client
        .head(latest_url.clone())
        .send()
        .context("fetch GitHub latest release page")?
        .error_for_status()
        .context("GitHub latest release page returned error")?;
    version_from_release_page_url(&latest_url, response.url())
}

fn version_from_release_page_url(latest_url: &reqwest::Url, release_url: &reqwest::Url) -> Result<String> {
    anyhow::ensure!(
        release_url.origin() == latest_url.origin(),
        "GitHub latest release redirected to an unexpected origin"
    );
    let releases_path = latest_url
        .path()
        .strip_suffix("/latest")
        .context("invalid latest release path")?;
    let tag_prefix = format!("{releases_path}/tag/");
    let tag = release_url
        .path()
        .strip_prefix(&tag_prefix)
        .context("GitHub latest release did not redirect to this repository's release tag")?;
    anyhow::ensure!(
        release_url.query().is_none() && release_url.fragment().is_none(),
        "unexpected query or fragment in GitHub release tag URL"
    );
    validated_release_version(tag)
}

fn validated_release_version(tag: &str) -> Result<String> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let parts: Vec<_> = version.split('.').collect();
    anyhow::ensure!(
        parts.len() == 3
            && parts.iter().all(|part| {
                !part.is_empty()
                    && part.bytes().all(|byte| byte.is_ascii_digit())
                    && (part.len() == 1 || !part.starts_with('0'))
                    && part.parse::<u32>().is_ok()
            }),
        "invalid GitHub release tag; expected vMAJOR.MINOR.PATCH"
    );
    Ok(version.to_string())
}

pub fn release_tag(version: &str) -> String {
    format!("v{}", normalize_version(version))
}

pub fn normalize_version(raw: &str) -> String {
    raw.trim().trim_start_matches('v').to_string()
}

pub fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    let left_parts = parse_version_parts(left);
    let right_parts = parse_version_parts(right);
    left_parts.cmp(&right_parts)
}

pub fn is_upgrade_available(latest: &str, current: &str) -> bool {
    compare_versions(latest, current) == std::cmp::Ordering::Greater
}

fn parse_version_parts(raw: &str) -> (u32, u32, u32) {
    let normalized = normalize_version(raw);
    let mut nums = normalized.split('.').map(|part| part.parse::<u32>().unwrap_or(0));
    (
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
    )
}

pub fn download_release_archive(repo: &str, tag: &str, platform: &Platform) -> Result<Vec<u8>> {
    let (archive_name, _) = archive_names(tag, platform);
    let url = format!("https://github.com/{repo}/releases/download/{tag}/{archive_name}");
    let client = Client::builder().user_agent(USER_AGENT).build()?;
    let response = client
        .get(&url)
        .send()
        .with_context(|| format!("download {url}"))?
        .error_for_status()
        .with_context(|| format!("release asset not found: {archive_name}"))?;
    Ok(response.bytes().context("read release archive")?.to_vec())
}

pub fn verify_checksum(repo: &str, tag: &str, platform: &Platform, archive_bytes: &[u8]) -> Result<()> {
    let (archive_name, _) = archive_names(tag, platform);
    let sums_url = format!("https://github.com/{repo}/releases/download/{tag}/SHA256SUMS.txt");
    let client = Client::builder().user_agent(USER_AGENT).build()?;
    let response = client.get(&sums_url).send();
    let Ok(response) = response else {
        return Ok(());
    };
    let Ok(response) = response.error_for_status() else {
        return Ok(());
    };
    let sums = response.text().unwrap_or_default();
    let expected = sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?;
        (name == archive_name).then_some(hash.to_string())
    });
    let Some(expected) = expected else {
        return Ok(());
    };
    let digest = Sha256::digest(archive_bytes);
    let actual = digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    if actual != expected {
        bail!("SHA256 mismatch for {archive_name}");
    }
    Ok(())
}

fn archive_names(tag: &str, platform: &Platform) -> (String, bool) {
    let version = normalize_version(tag);
    if platform.slug == "windows-x86_64" {
        (format!("{BIN_NAME}-{version}-{}.zip", platform.slug), true)
    } else {
        (format!("{BIN_NAME}-{version}-{}.tar.gz", platform.slug), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use std::thread;
    use std::time::Duration;

    #[test]
    fn api_rate_limit_falls_back_to_latest_release_redirect() {
        let server = TestServer::spawn(vec![
            "HTTP/1.1 403 Forbidden\r\nX-RateLimit-Remaining: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            "HTTP/1.1 302 Found\r\nLocation: /owner/repo/releases/tag/v0.5.0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        ]);
        let client = Client::builder().timeout(Duration::from_secs(2)).build().unwrap();
        let version = fetch_latest_version_from(
            &client,
            &format!("{}/api/releases/latest", server.base_url),
            &format!("{}/owner/repo/releases/latest", server.base_url),
        )
        .unwrap();
        assert_eq!(version, "0.5.0");
        assert_eq!(
            server.requests.lock().unwrap().as_slice(),
            [
                "GET /api/releases/latest HTTP/1.1",
                "HEAD /owner/repo/releases/latest HTTP/1.1",
                "HEAD /owner/repo/releases/tag/v0.5.0 HTTP/1.1",
            ]
        );
    }

    #[test]
    fn api_success_does_not_request_the_release_page() {
        let body = r#"{"tag_name":"v0.5.0"}"#;
        let server = TestServer::spawn(vec![format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )]);
        let client = Client::builder().timeout(Duration::from_secs(2)).build().unwrap();
        assert_eq!(
            fetch_latest_version_from(
                &client,
                &format!("{}/api/releases/latest", server.base_url),
                &format!("{}/owner/repo/releases/latest", server.base_url),
            )
            .unwrap(),
            "0.5.0"
        );
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn release_page_fallback_rejects_another_repository() {
        let server = TestServer::spawn(vec![
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            "HTTP/1.1 302 Found\r\nLocation: /another/repo/releases/tag/v9.9.9\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        ]);
        let client = Client::builder().timeout(Duration::from_secs(2)).build().unwrap();
        let error = fetch_latest_version_from(
            &client,
            &format!("{}/api/releases/latest", server.base_url),
            &format!("{}/owner/repo/releases/latest", server.base_url),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("403 Forbidden"), "{error}");
        assert!(error.contains("this repository's release tag"), "{error}");
    }

    #[test]
    fn failed_api_and_release_page_both_appear_in_error() {
        let server = TestServer::spawn(vec![
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        ]);
        let client = Client::builder().timeout(Duration::from_secs(2)).build().unwrap();
        let error = fetch_latest_version_from(
            &client,
            &format!("{}/api/releases/latest", server.base_url),
            &format!("{}/owner/repo/releases/latest", server.base_url),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("API lookup:"), "{error}");
        assert!(error.contains("release page lookup:"), "{error}");
        assert!(error.contains("403 Forbidden"), "{error}");
        assert!(error.contains("503 Service Unavailable"), "{error}");
    }

    #[test]
    fn release_page_url_requires_same_origin_and_a_valid_stable_tag() {
        let latest = reqwest::Url::parse("https://github.com/owner/repo/releases/latest").unwrap();
        for invalid in [
            "https://example.com/owner/repo/releases/tag/v0.5.0",
            "http://github.com/owner/repo/releases/tag/v0.5.0",
            "https://github.com:444/owner/repo/releases/tag/v0.5.0",
            "https://github.com/owner/repo/releases/latest",
            "https://github.com/other/repo/releases/tag/v0.5.0",
            "https://github.com/owner/repo/releases/tag/v0.5.0/other",
            "https://github.com/owner/repo/releases/tag/v0.5.0?download=1",
            "https://github.com/owner/repo/releases/tag/v0.5.0#asset",
            "https://github.com/owner/repo/releases/tag/v0.5",
            "https://github.com/owner/repo/releases/tag/v0.5.0-rc.1",
        ] {
            assert!(
                version_from_release_page_url(&latest, &reqwest::Url::parse(invalid).unwrap()).is_err(),
                "{invalid}"
            );
        }
        for invalid in ["", "v", "latest", "vv0.5.0", "v01.5.0", "v0.5.0+build", "v0.5.0/other"] {
            assert!(validated_release_version(invalid).is_err(), "{invalid}");
        }
        let release = reqwest::Url::parse("https://github.com/owner/repo/releases/tag/v0.5.0").unwrap();
        assert_eq!(version_from_release_page_url(&latest, &release).unwrap(), "0.5.0");
    }

    struct TestServer {
        base_url: String,
        requests: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn spawn(responses: Vec<String>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let request_log = requests.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let stop_thread = stop.clone();
            let thread = thread::spawn(move || {
                let mut responses: VecDeque<_> = responses.into();
                while !stop_thread.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                            let mut request = Vec::new();
                            let mut buffer = [0; 2048];
                            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                                let length = stream.read(&mut buffer).unwrap();
                                if length == 0 {
                                    break;
                                }
                                request.extend_from_slice(&buffer[..length]);
                            }
                            request_log
                                .lock()
                                .unwrap()
                                .push(String::from_utf8_lossy(&request).lines().next().unwrap().to_string());
                            let response = responses.pop_front().unwrap_or_else(|| {
                                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
                            });
                            stream.write_all(response.as_bytes()).unwrap();
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => panic!("accept test request: {error}"),
                    }
                }
            });
            Self { base_url, requests, stop, thread: Some(thread) }
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            self.thread.take().unwrap().join().unwrap();
        }
    }

    #[test]
    fn compare_versions_orders_semver_triplets() {
        assert_eq!(compare_versions("0.3.2", "0.3.1"), std::cmp::Ordering::Greater);
        assert_eq!(compare_versions("0.3.1", "0.3.2"), std::cmp::Ordering::Less);
        assert_eq!(compare_versions("1.0.0", "1.0.0"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn normalize_strips_v_prefix() {
        assert_eq!(normalize_version("v0.3.2"), "0.3.2");
    }
}
