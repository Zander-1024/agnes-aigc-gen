use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::platform::Platform;

pub const DEFAULT_REPO: &str = "Zander-1024/agnes-aigc-gen";
const BIN_NAME: &str = "agnes-aigc-gen";
const USER_AGENT: &str = concat!("agnes-aigc-gen/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Deserialize)]
struct LatestRelease {
    tag_name: String,
}

pub fn fetch_latest_version(repo: &str) -> Result<String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let client = Client::builder().user_agent(USER_AGENT).build()?;
    let response = client
        .get(url)
        .send()
        .context("fetch latest GitHub release")?
        .error_for_status()
        .context("GitHub releases/latest returned error")?;
    let release: LatestRelease = response.json().context("parse latest release JSON")?;
    Ok(normalize_version(&release.tag_name))
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
