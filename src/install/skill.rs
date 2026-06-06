use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use reqwest::blocking::Client;

pub const SKILL_NAME: &str = "agnes-aigc-gen";
const USER_AGENT: &str = concat!("agnes-aigc-gen/", env!("CARGO_PKG_VERSION"));
const SKILL_FILE: &str = "SKILL.md";
const SETUP_FILE: &str = "SETUP.md";

pub fn known_skill_parents() -> Result<Vec<PathBuf>> {
    let Some(home) = dirs::home_dir() else {
        return Ok(Vec::new());
    };
    Ok(vec![
        home.join(".agents/skills"),
        home.join(".codex/skills"),
        home.join(".cursor/skills"),
        home.join(".claude/skills"),
        home.join(".openclaw/skills"),
        home.join(".hermes/skills"),
    ])
}

pub fn default_skill_parent() -> Result<String> {
    let home = dirs::home_dir().context("resolve home directory")?;
    Ok(home.join(".agents/skills").display().to_string())
}

pub fn install_skill_from_release(repo: &str, tag: &str, parent_dirs: &[String]) -> Result<()> {
    let client = Client::builder().user_agent(USER_AGENT).build()?;
    for parent in parent_dirs {
        let dest = Path::new(parent).join(SKILL_NAME);
        fs::create_dir_all(&dest).with_context(|| format!("create {}", dest.display()))?;
        for (file, url) in [
            (
                SKILL_FILE,
                format!("https://raw.githubusercontent.com/{repo}/{tag}/skills/{SKILL_NAME}/{SKILL_FILE}"),
            ),
            (
                SETUP_FILE,
                format!("https://raw.githubusercontent.com/{repo}/{tag}/docs/{SETUP_FILE}"),
            ),
        ] {
            let response = client
                .get(&url)
                .send()
                .with_context(|| format!("download {url}"))?
                .error_for_status()
                .with_context(|| format!("skill file not found: {file}"))?;
            let bytes = response.bytes().context("read skill file")?;
            let out = dest.join(file);
            fs::write(&out, bytes).with_context(|| format!("write {}", out.display()))?;
            println!("Updated skill {}", out.display());
        }
    }
    Ok(())
}
