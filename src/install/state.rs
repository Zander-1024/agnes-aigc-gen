use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::skill::{SKILL_NAME, default_skill_parent, known_skill_parents};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTarget {
    pub parent_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallState {
    pub version: u32,
    pub installed_version: String,
    pub binary_path: PathBuf,
    #[serde(default)]
    pub skill_targets: Vec<SkillTarget>,
}

impl Default for InstallState {
    fn default() -> Self {
        Self {
            version: 1,
            installed_version: super::PKG_VERSION.to_string(),
            binary_path: PathBuf::new(),
            skill_targets: Vec::new(),
        }
    }
}

impl InstallState {
    pub fn path() -> Result<PathBuf> {
        Ok(crate::config::AppConfig::config_dir()?.join("install.toml"))
    }

    pub fn load_or_discover() -> Result<Self> {
        let path = Self::path()?;
        if path.exists() {
            let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
            let mut state: Self = toml::from_str(&raw).context("parse install.toml")?;
            if state.binary_path.as_os_str().is_empty() {
                state.binary_path = std::env::current_exe().context("resolve current executable")?;
            }
            return Ok(state);
        }
        Self::discover()
    }

    pub fn discover() -> Result<Self> {
        let binary_path = std::env::current_exe().context("resolve current executable")?;
        let skill_targets = discover_skill_targets()?
            .into_iter()
            .map(|parent_dir| SkillTarget { parent_dir })
            .collect();
        Ok(Self { version: 1, installed_version: super::PKG_VERSION.to_string(), binary_path, skill_targets })
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self).context("serialize install.toml")?;
        fs::write(&path, raw).with_context(|| format!("write {}", path.display()))
    }

    pub fn recommended_skill_parents(&self) -> Vec<String> {
        if self.skill_targets.is_empty() {
            default_skill_parent().into_iter().collect()
        } else {
            self.skill_targets
                .iter()
                .map(|target| target.parent_dir.clone())
                .collect()
        }
    }

    pub fn skill_install_dirs(&self) -> Vec<PathBuf> {
        self.recommended_skill_parents()
            .into_iter()
            .map(|parent| PathBuf::from(parent).join(SKILL_NAME))
            .collect()
    }
}

fn discover_skill_targets() -> Result<Vec<String>> {
    let mut found = Vec::new();
    for parent in known_skill_parents()? {
        let skill_dir = parent.join(SKILL_NAME);
        let skill_file = skill_dir.join("SKILL.md");
        if skill_file.is_file() {
            found.push(parent.display().to_string());
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_state_roundtrip_toml() {
        let state = InstallState {
            version: 1,
            installed_version: "0.4.0".into(),
            binary_path: PathBuf::from("/home/user/.local/bin/agnes-aigc-gen"),
            skill_targets: vec![SkillTarget { parent_dir: "/home/user/.agents/skills".into() }],
        };
        let raw = toml::to_string_pretty(&state).unwrap();
        let parsed: InstallState = toml::from_str(&raw).unwrap();
        assert_eq!(parsed.installed_version, "0.4.0");
        assert_eq!(parsed.skill_targets.len(), 1);
    }
}
