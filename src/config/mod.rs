mod defaults;

pub use defaults::*;

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};

use crate::crypto;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub base_url: String,
    pub text_model: String,
    pub image_model: String,
    pub video_model: String,
    pub output_dir: String,
    pub save_local: bool,
    pub max_retries: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_encrypted: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            text_model: DEFAULT_TEXT_MODEL.to_string(),
            image_model: DEFAULT_IMAGE_MODEL.to_string(),
            video_model: DEFAULT_VIDEO_MODEL.to_string(),
            output_dir: DEFAULT_OUTPUT_DIR.to_string(),
            save_local: false,
            max_retries: DEFAULT_MAX_RETRIES,
            api_key_encrypted: None,
        }
    }
}

impl AppConfig {
    pub fn config_dir() -> Result<PathBuf> {
        let dir = dirs::config_dir()
            .context("could not resolve config directory")?
            .join("agnes-aigc-gen");
        Ok(dir)
    }

    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.toml"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        toml::from_str(&raw).context("parse config.toml")
    }

    pub fn save(&self) -> Result<()> {
        let dir = Self::config_dir()?;
        fs::create_dir_all(&dir)?;
        let path = dir.join("config.toml");
        let raw = toml::to_string_pretty(self).context("serialize config")?;
        fs::write(&path, raw).with_context(|| format!("write {}", path.display()))
    }

    pub fn resolved_output_dir(&self) -> Result<PathBuf> {
        let path = expand_tilde(&self.output_dir)?;
        if path.as_os_str() == "." {
            return std::env::current_dir().context("resolve current directory for output_dir");
        }
        if path.is_relative() {
            return Ok(std::env::current_dir()?.join(path));
        }
        Ok(path)
    }

    pub fn api_key(&self) -> Result<String> {
        let encrypted = self
            .api_key_encrypted
            .as_ref()
            .context("api key not configured; run: agnes-aigc-gen config set api-key <KEY>")?;
        crypto::decrypt_api_key(encrypted)
    }

    pub fn set_api_key(&mut self, plain: &str) -> Result<()> {
        let encrypted = crypto::encrypt_api_key(plain)?;
        self.api_key_encrypted = Some(encrypted);
        Ok(())
    }

    pub fn apply_key(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "base-url" | "base_url" => self.base_url = value.to_string(),
            "text-model" | "text_model" => self.text_model = value.to_string(),
            "image-model" | "image_model" => self.image_model = value.to_string(),
            "video-model" | "video_model" => self.video_model = value.to_string(),
            "output-dir" | "output_dir" => self.output_dir = value.to_string(),
            "save-local" | "save_local" => {
                self.save_local = parse_bool(value)?;
            }
            "max-retries" | "max_retries" => {
                self.max_retries = value.parse().context("max-retries must be a number")?;
            }
            "api-key" | "api_key" => self.set_api_key(value)?,
            other => {
                print_settable_keys(Some(other))?;
                bail!("unknown config key: {other}");
            }
        }
        Ok(())
    }
}

struct ConfigKeyInfo {
    names: &'static str,
    description: &'static str,
    example: &'static str,
}

const SETTABLE_KEYS: &[ConfigKeyInfo] = &[
    ConfigKeyInfo { names: "api-key", description: "API key (encrypted)", example: "sk-..." },
    ConfigKeyInfo { names: "base-url", description: "API gateway", example: DEFAULT_BASE_URL },
    ConfigKeyInfo { names: "text-model", description: "Text model", example: DEFAULT_TEXT_MODEL },
    ConfigKeyInfo { names: "image-model", description: "Image model", example: DEFAULT_IMAGE_MODEL },
    ConfigKeyInfo { names: "video-model", description: "Video model", example: DEFAULT_VIDEO_MODEL },
    ConfigKeyInfo { names: "output-dir", description: "Output dir (`.` = cwd)", example: "." },
    ConfigKeyInfo {
        names: "save-local",
        description: "Download outputs locally (default: remote URL only)",
        example: "true",
    },
    ConfigKeyInfo { names: "max-retries", description: "Retry count", example: "3" },
];

fn print_settable_keys(highlight: Option<&str>) -> Result<()> {
    let cfg = AppConfig::load().unwrap_or_default();
    let highlight = highlight.map(|s| normalize_key(s));

    if let Some(ref key) = highlight {
        if let Some(info) = SETTABLE_KEYS.iter().find(|k| key_matches(k, key)) {
            eprintln!(
                "Missing value. Example:\n  agnes-aigc-gen config set {} {}",
                info.names, info.example
            );
            return Ok(());
        }
        eprintln!("Unknown key: {key}\n");
    }

    println!("Usage: agnes-aigc-gen config set <KEY> <VALUE>\n");
    for info in SETTABLE_KEYS {
        let current = truncate_display(current_value_for_key(&cfg, info.names), 26);
        println!("  {:<14} {:<26} {}", info.names, current, info.description);
    }
    Ok(())
}

fn truncate_display(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        s
    } else {
        format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}

fn key_matches(info: &ConfigKeyInfo, key: &str) -> bool {
    normalize_key(info.names) == normalize_key(key)
}

fn normalize_key(key: &str) -> String {
    key.replace('_', "-").to_lowercase()
}

fn current_value_for_key(cfg: &AppConfig, key: &str) -> String {
    match key {
        "api-key" => {
            if cfg.api_key_encrypted.is_some() {
                "<configured>".into()
            } else {
                "<not set>".into()
            }
        }
        "base-url" => cfg.base_url.clone(),
        "text-model" => cfg.text_model.clone(),
        "image-model" => cfg.image_model.clone(),
        "video-model" => cfg.video_model.clone(),
        "output-dir" => cfg.output_dir.clone(),
        "save-local" => cfg.save_local.to_string(),
        "max-retries" => cfg.max_retries.to_string(),
        _ => "?".into(),
    }
}

pub fn expand_tilde(path: &str) -> Result<PathBuf> {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = dirs::home_dir().context("could not resolve home directory")?;
        Ok(home.join(rest))
    } else if path == "~" {
        dirs::home_dir().context("could not resolve home directory")
    } else {
        Ok(PathBuf::from(path))
    }
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => bail!("expected boolean, got {value}"),
    }
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Show current configuration
    Show,
    /// Set a configuration value (incremental merge). Run without arguments to list keys.
    Set {
        /// Config key (e.g. api-key, base-url). Omit to list all keys.
        key: Option<String>,
        /// Value to assign. Required when key is provided.
        value: Option<String>,
    },
}

#[derive(Args)]
pub struct ConfigCmd {
    #[command(subcommand)]
    pub action: ConfigAction,
}

pub fn run(cmd: ConfigCmd) -> Result<()> {
    match cmd.action {
        ConfigAction::Show => {
            let cfg = AppConfig::load()?;
            let resolved_out = cfg.resolved_output_dir().ok();
            println!("base_url     = {}  ({BASE_URL_HELP})", cfg.base_url);
            println!("text_model   = {}", cfg.text_model);
            println!("image_model  = {}", cfg.image_model);
            println!("video_model  = {}", cfg.video_model);
            println!("output_dir   = {}  ({OUTPUT_DIR_HELP})", cfg.output_dir);
            if let Some(path) = resolved_out {
                println!("               resolved: {}", path.display());
            }
            println!("save_local   = {}", cfg.save_local);
            println!("max_retries  = {}", cfg.max_retries);
            println!(
                "api_key      = {}",
                if cfg.api_key_encrypted.is_some() {
                    "<configured>"
                } else {
                    "<not set>"
                }
            );
        }
        ConfigAction::Set { key, value } => match (&key, &value) {
            (None, _) => {
                print_settable_keys(None)?;
                return Ok(());
            }
            (Some(k), None) => {
                print_settable_keys(Some(k))?;
                return Ok(());
            }
            (Some(k), Some(v)) => {
                let mut cfg = AppConfig::load()?;
                cfg.apply_key(k, v)?;
                cfg.save()?;
                println!("updated {k}");
            }
        },
    }
    Ok(())
}
