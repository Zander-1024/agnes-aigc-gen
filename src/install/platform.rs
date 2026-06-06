use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub slug: String,
}

pub fn detect_platform() -> Result<Platform> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let slug = match (os, arch) {
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("macos", "aarch64") => "darwin-aarch64",
        ("windows", "x86_64") => "windows-x86_64",
        _ => bail!("unsupported platform: {os}-{arch}"),
    };
    Ok(Platform { slug: slug.to_string() })
}

pub fn platform_slug(platform: &Platform) -> &str {
    &platform.slug
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_platform_returns_slug() {
        let platform = detect_platform().unwrap();
        assert!(!platform.slug.is_empty());
    }
}
