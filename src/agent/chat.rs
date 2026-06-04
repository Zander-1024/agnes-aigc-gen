use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use pi_agent::PermissionPolicy;
use pi_ai::{Model, StreamOptions};

use crate::config::AppConfig;

use super::runner::AgnesAgentConfig;
use super::tools::default_agent_tools;

#[derive(Debug, Clone, Default)]
pub struct ChatOverrides {
    pub thinking: Option<bool>,
    pub context_tokens: Option<u32>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ChatRuntimeConfig {
    pub model: Model,
    pub stream: StreamOptions,
    pub enable_thinking: bool,
}

pub const BASE_CHAT_SYSTEM_PROMPT: &str = r#"You are Agnes Chat, a PI-based terminal coding and media agent.

You can read, edit, search, run commands, manage todos, load skills, and call Agnes image/video tools.

Important Agnes media rules:
- Use agnes_generate_image for image generation or image editing.
- Use agnes_submit_video for video generation. It submits asynchronously and returns a local task id; do not block the main conversation waiting for video completion unless the user asks.
- Video image inputs must be HTTPS URLs or asset:// references. Do not pass local paths, base64, or data URIs to video tools.
- Image inputs may be local path, HTTPS URL, asset://, base64, or data URI.
- Prefer asset:// references when chaining image outputs into video.

Approval rules:
- The runtime may ask the user before tool calls. If a tool is denied, explain what you could do instead.
- Dangerous commands always require human review even in automatic mode.

Be concise, inspect before changing code, and keep edits focused.
"#;

impl ChatRuntimeConfig {
    pub fn from_app_config(config: &AppConfig, overrides: ChatOverrides) -> Result<Self> {
        let enable_thinking = overrides.thinking.unwrap_or(config.chat_thinking);
        let context_tokens = overrides.context_tokens.unwrap_or(config.chat_context_tokens);
        let max_output_tokens = overrides.max_output_tokens.unwrap_or(config.chat_max_output_tokens);
        let model_id = if enable_thinking {
            config
                .thinking_text_model
                .as_deref()
                .unwrap_or(config.text_model.as_str())
        } else {
            config.text_model.as_str()
        };
        let model = Model::openai_compat(
            "agnes",
            model_id,
            config.base_url.clone(),
            context_tokens,
            max_output_tokens,
        );
        let stream = StreamOptions {
            max_tokens: Some(max_output_tokens),
            base_url: Some(config.base_url.clone()),
            ..StreamOptions::default()
        };
        Ok(Self { model, stream, enable_thinking })
    }

    pub fn with_api_key(mut self, api_key: String) -> Self {
        self.stream.api_key = Some(api_key);
        self
    }
}

pub fn build_agent_config(
    app_config: &AppConfig,
    overrides: ChatOverrides,
    max_turns: u32,
    permission: Arc<dyn PermissionPolicy>,
    skill_context: &[String],
) -> Result<AgnesAgentConfig> {
    let runtime = ChatRuntimeConfig::from_app_config(app_config, overrides)?.with_api_key(app_config.api_key()?);
    Ok(AgnesAgentConfig {
        system_prompt: build_system_prompt(skill_context)?,
        runtime,
        tools: default_agent_tools(),
        max_turns,
        permission,
    })
}

pub fn build_system_prompt(skill_context: &[String]) -> Result<String> {
    let mut prompt = BASE_CHAT_SYSTEM_PROMPT.to_string();
    let project = load_project_instructions()?;
    if !project.is_empty() {
        prompt.push_str("\n----- project instructions -----\n");
        prompt.push_str(&project);
    }
    if !skill_context.is_empty() {
        prompt.push_str("\n----- loaded skills -----\n");
        for skill in skill_context {
            prompt.push_str(skill);
            if !skill.ends_with('\n') {
                prompt.push('\n');
            }
        }
    }
    Ok(prompt)
}

fn load_project_instructions() -> Result<String> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let mut paths = Vec::new();
    collect_project_instruction_paths(&cwd, &mut paths);
    let mut out = String::new();
    for path in paths {
        if let Ok(text) = std::fs::read_to_string(&path) {
            out.push_str(&format!("\n## {}\n", path.display()));
            out.push_str(&text);
            if !text.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    Ok(out)
}

fn collect_project_instruction_paths(cwd: &Path, paths: &mut Vec<PathBuf>) {
    let mut current = Some(cwd);
    while let Some(dir) = current {
        for name in ["AGENTS.md", "CLAUDE.md", ".pi/instructions.md"] {
            let path = dir.join(name);
            if path.exists() {
                paths.push(path);
            }
        }
        current = dir.parent();
    }
    paths.reverse();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn default_chat_model_uses_thinking_fallback_and_configured_limits() {
        let cfg = AppConfig::default();
        let runtime = ChatRuntimeConfig::from_app_config(&cfg, ChatOverrides::default()).unwrap();

        assert!(runtime.enable_thinking);
        assert_eq!(runtime.model.id, cfg.text_model);
        assert_eq!(runtime.model.context_window, 262_144);
        assert_eq!(runtime.model.max_tokens, 65_536);
        assert_eq!(runtime.stream.max_tokens, Some(65_536));
    }

    #[test]
    fn non_thinking_override_uses_text_model() {
        let cfg = AppConfig {
            text_model: "agnes-non-thinking".into(),
            thinking_text_model: Some("agnes-thinking".into()),
            ..AppConfig::default()
        };
        let runtime = ChatRuntimeConfig::from_app_config(
            &cfg,
            ChatOverrides { thinking: Some(false), ..ChatOverrides::default() },
        )
        .unwrap();

        assert!(!runtime.enable_thinking);
        assert_eq!(runtime.model.id, "agnes-non-thinking");
    }

    #[test]
    fn thinking_override_uses_thinking_model_when_configured() {
        let cfg = AppConfig {
            text_model: "agnes-non-thinking".into(),
            thinking_text_model: Some("agnes-thinking".into()),
            ..AppConfig::default()
        };
        let runtime = ChatRuntimeConfig::from_app_config(
            &cfg,
            ChatOverrides { thinking: Some(true), context_tokens: Some(4096), max_output_tokens: Some(2048) },
        )
        .unwrap();

        assert!(runtime.enable_thinking);
        assert_eq!(runtime.model.id, "agnes-thinking");
        assert_eq!(runtime.model.context_window, 4096);
        assert_eq!(runtime.model.max_tokens, 2048);
        assert_eq!(runtime.stream.max_tokens, Some(2048));
    }
}
