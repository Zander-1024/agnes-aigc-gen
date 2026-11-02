use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use pi_agent::PermissionPolicy;
use pi_ai::{Model, StreamOptions};
use serde_json::{Value, json};

use crate::config::{AppConfig, RequestKind};

use super::agnes_stream::AgnesStreamOptions;
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
    pub request_body: Value,
    pub request_config_hash: String,
    pub request_layers: Vec<String>,
    pub session_id: Option<String>,
}

pub const BASE_CHAT_SYSTEM_PROMPT: &str = r#"You are Agnes Chat, a PI-based terminal coding and media agent.

You can read, edit, search, run commands, manage todos, load skills, and call Agnes image/video tools.

Important Agnes media rules:
- Use agnes_generate_image for image generation or image editing.
- Use agnes_submit_video for video generation. It submits asynchronously and returns a local task id; do not block the main conversation waiting for video completion unless the user asks.
- Video first/last frames and reference images accept HTTPS URLs, asset://, local paths, Base64, and Data URIs. Inline and local images are validated and sent as JPEG Data URIs.
- Video audio and video references require HTTPS URLs or asset:// references.
- Image inputs may be local path, HTTPS URL, asset://, base64, or data URI.
- Prefer asset:// references when chaining image outputs into video.

Approval rules:
- The runtime may ask the user before tool calls. If a tool is denied, explain what you could do instead.
- Dangerous commands always require human review even in automatic mode.

Be concise, inspect before changing code, and keep edits focused.
"#;

impl ChatRuntimeConfig {
    pub fn from_app_config(config: &AppConfig, overrides: ChatOverrides) -> Result<Self> {
        let selected_thinking = overrides.thinking.unwrap_or(config.chat_thinking);
        let context_tokens = overrides.context_tokens.unwrap_or(config.chat_context_tokens);
        let model_id = if selected_thinking {
            config
                .thinking_text_model
                .as_deref()
                .unwrap_or(config.text_model.as_str())
        } else {
            config.text_model.as_str()
        };
        let mut base = json!({"max_tokens": config.chat_max_output_tokens});
        if config.chat_thinking {
            base["chat_template_kwargs"] = json!({"enable_thinking": true});
        }
        let mut explicit = json!({});
        if let Some(thinking) = overrides.thinking {
            explicit["chat_template_kwargs"] = json!({"enable_thinking": thinking});
        }
        if let Some(tokens) = overrides.max_output_tokens {
            explicit["max_tokens"] = json!(tokens);
        }
        let resolved = config
            .request_params
            .resolve(RequestKind::Text, model_id, &base, &explicit)?;
        let max_output_tokens = resolved.body["max_tokens"]
            .as_u64()
            .and_then(|tokens| u32::try_from(tokens).ok())
            .filter(|tokens| *tokens > 0)
            .context("text max_tokens must be a positive 32-bit integer")?;
        let enable_thinking = match resolved.body.get("chat_template_kwargs") {
            Some(kwargs) => {
                anyhow::ensure!(kwargs.is_object(), "text chat_template_kwargs must be an object");
                match kwargs.get("enable_thinking") {
                    Some(value) => value.as_bool().context("text enable_thinking must be a boolean")?,
                    None => selected_thinking,
                }
            }
            None => selected_thinking,
        };
        for key in ["temperature", "top_p"] {
            if let Some(value) = resolved.body.get(key) {
                anyhow::ensure!(
                    value.is_number() || value.is_null(),
                    "text {key} must be a number or null"
                );
            }
        }
        let base_url = crate::api::endpoints::v1_endpoint(&config.base_url, "")?;
        let model = Model::openai_compat("agnes", model_id, base_url.clone(), context_tokens, max_output_tokens);
        let stream = StreamOptions {
            max_tokens: Some(max_output_tokens),
            temperature: resolved
                .body
                .get("temperature")
                .and_then(Value::as_f64)
                .map(|value| value as f32),
            base_url: Some(base_url),
            ..StreamOptions::default()
        };
        Ok(Self {
            model,
            stream,
            enable_thinking,
            request_body: resolved.body,
            request_config_hash: resolved.config_hash,
            request_layers: resolved.layers,
            session_id: None,
        })
    }

    pub fn stream_options(&self) -> AgnesStreamOptions {
        AgnesStreamOptions {
            stream: self.stream.clone(),
            enable_thinking: self.enable_thinking,
            request_body: self.request_body.clone(),
            request_config_hash: self.request_config_hash.clone(),
            request_layers: self.request_layers.clone(),
            record_requests: true,
            session_id: self.session_id.clone(),
            turn: None,
        }
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
        assert_eq!(runtime.model.context_window, 524_288);
        assert_eq!(runtime.model.max_tokens, 65_536);
        assert_eq!(runtime.stream.max_tokens, Some(65_536));
    }

    #[test]
    fn chat_runtime_normalizes_gateway_and_versioned_base_urls() {
        for base_url in
            ["https://example.com", "https://example.com/", "https://example.com/v1", "https://example.com/v1/"]
        {
            let cfg = AppConfig { base_url: base_url.into(), ..AppConfig::default() };
            let runtime = ChatRuntimeConfig::from_app_config(&cfg, ChatOverrides::default()).unwrap();
            assert_eq!(runtime.model.base_url, "https://example.com/v1");
            assert_eq!(runtime.stream.base_url.as_deref(), Some("https://example.com/v1"));
        }

        let cfg = AppConfig { base_url: "https://example.com/proxy/v1/".into(), ..AppConfig::default() };
        let runtime = ChatRuntimeConfig::from_app_config(&cfg, ChatOverrides::default()).unwrap();
        assert_eq!(runtime.model.base_url, "https://example.com/proxy/v1");
        assert_eq!(runtime.stream.base_url.as_deref(), Some("https://example.com/proxy/v1"));
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

    #[test]
    fn configured_text_defaults_update_runtime_and_keep_nested_siblings() {
        let cfg = AppConfig {
            request_params: serde_json::from_value(json!({
                "text": {
                    "body": {
                        "temperature": 0.4,
                        "max_tokens": 1200,
                        "chat_template_kwargs": {"enable_thinking": false, "vendor_mode": "keep"},
                        "vendor_option": {"enabled": true}
                    },
                    "models": {"agnes-3.0-flash": {"body": {"top_p": 0.8}}}
                }
            }))
            .unwrap(),
            ..AppConfig::default()
        };
        let runtime = ChatRuntimeConfig::from_app_config(&cfg, ChatOverrides::default()).unwrap();
        assert!(!runtime.enable_thinking);
        assert_eq!(runtime.model.max_tokens, 1200);
        assert_eq!(runtime.stream.max_tokens, Some(1200));
        assert_eq!(runtime.stream.temperature, Some(0.4));
        assert_eq!(runtime.request_body["chat_template_kwargs"]["vendor_mode"], "keep");
        assert_eq!(runtime.request_body["top_p"], 0.8);

        let explicit = ChatRuntimeConfig::from_app_config(
            &cfg,
            ChatOverrides { thinking: Some(true), max_output_tokens: Some(2048), ..ChatOverrides::default() },
        )
        .unwrap();
        assert!(explicit.enable_thinking);
        assert_eq!(explicit.model.max_tokens, 2048);
        assert_eq!(explicit.request_body["chat_template_kwargs"]["enable_thinking"], true);
        assert_eq!(explicit.request_body["chat_template_kwargs"]["vendor_mode"], "keep");
        assert_eq!(explicit.request_body["vendor_option"]["enabled"], true);
        assert_eq!(explicit.request_body["temperature"], 0.4);
    }

    #[test]
    fn invalid_text_defaults_fail_during_runtime_setup() {
        for body in [
            json!({"max_tokens": 0}),
            json!({"max_tokens": "100"}),
            json!({"chat_template_kwargs": false}),
            json!({"chat_template_kwargs": {"enable_thinking": "false"}}),
            json!({"temperature": "0.4"}),
            json!({"messages": []}),
            json!({"stream": false}),
        ] {
            let cfg = AppConfig {
                request_params: serde_json::from_value(json!({"text": {"body": body}})).unwrap(),
                ..AppConfig::default()
            };
            assert!(ChatRuntimeConfig::from_app_config(&cfg, ChatOverrides::default()).is_err());
        }
    }

    #[test]
    fn stream_options_carry_the_associated_chat_session() {
        let mut runtime = ChatRuntimeConfig::from_app_config(&AppConfig::default(), ChatOverrides::default()).unwrap();
        assert!(runtime.session_id.is_none());
        runtime.session_id = Some("session-fixture".into());
        let options = runtime.stream_options();
        assert_eq!(options.session_id.as_deref(), Some("session-fixture"));
        assert_eq!(options.turn, None);
        assert_eq!(runtime.session_id.as_deref(), Some("session-fixture"));
    }
}
