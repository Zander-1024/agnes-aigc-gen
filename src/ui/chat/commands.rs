use anyhow::Result;

use crate::agent::session::{ChatSession, list_sessions, load_session, save_session};
use crate::agent::tools::{default_agent_tools, list_available_skills};
use crate::config::AppConfig;
use crate::db::Database;

use super::model::{ChatLineKind, ChatUiOptions};
use super::skill_meta::skill_description_from_markdown;
use super::state::ChatUiState;
use super::status::{format_u64_tokens, truncate};

pub(crate) const SLASH_COMMANDS: &[&str] = &[
    "/help",
    "/new",
    "/sessions",
    "/resume",
    "/image",
    "/images",
    "/skills",
    "/skill",
    "/tools",
    "/approval",
    "/compress",
    "/model",
    "/thinking",
    "/tasks",
    "/retry",
    "/quit",
    "/exit",
];

pub(crate) fn command_hint_for_input(input: &str) -> Option<String> {
    if !input.starts_with('/') {
        return None;
    }
    if input.starts_with("/image ") {
        return Some("Attach one image: URL, local path, Base64, or Data URI; /images lists pending images".into());
    }
    if input.starts_with("/images ") {
        return Some("Use /images clear to remove pending images".into());
    }
    if let Some(prefix) = input.strip_prefix("/skill ") {
        let matches = skill_name_matches(prefix.trim());
        return match matches.as_slice() {
            [] => Some("No matching skill".into()),
            [only] => Some(format!("Tab completes skill: {only}")),
            many => Some(format!(
                "Tab completes skill: {}",
                many.iter().take(8).cloned().collect::<Vec<_>>().join("  ")
            )),
        };
    }
    let prefix = input.split_whitespace().next().unwrap_or("");
    let matches: Vec<&str> = SLASH_COMMANDS
        .iter()
        .copied()
        .filter(|command| command.starts_with(prefix))
        .collect();
    if matches.len() > 1 {
        Some(format!("Tab completes: {}", matches.join("  ")))
    } else {
        matches.first().map(|command| format!("Tab completes: {command}"))
    }
}

pub(crate) fn skill_name_matches(prefix: &str) -> Vec<String> {
    list_available_skills()
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| name.starts_with(prefix))
        .collect()
}

pub(crate) fn handle_slash_command(
    state: &mut ChatUiState,
    cfg: &AppConfig,
    _options: &ChatUiOptions,
    input: &str,
) -> Result<bool> {
    let mut parts = input.split_whitespace();
    let cmd = parts.next().unwrap_or("");
    if state.busy && matches!(cmd, "/new" | "/resume") {
        state.push_line(
            ChatLineKind::Error,
            "wait for the current request before changing sessions",
        );
        return Ok(false);
    }
    match cmd {
        "/quit" | "/exit" => return Ok(true),
        "/help" => {
            state.handle_slash(input);
        }
        "/new" => {
            state.session = Some(ChatSession::new(
                cfg.thinking_text_model.as_deref().unwrap_or(cfg.text_model.as_str()),
                state.overrides.thinking.unwrap_or(cfg.chat_thinking),
            ));
            state.lines.clear();
            state.pending_images.clear();
            state.last_request = None;
            state.push_line(ChatLineKind::Status, "new session");
        }
        "/sessions" => {
            for summary in list_sessions()?.into_iter().take(20) {
                state.push_line(
                    ChatLineKind::Status,
                    format!(
                        "{} | {} msgs | {} | thinking={} | {}",
                        summary.id,
                        summary.turns,
                        summary.model,
                        summary.thinking,
                        truncate(&summary.first_message, 60)
                    ),
                );
            }
        }
        "/resume" => {
            let Some(id) = parts.next() else {
                state.push_line(ChatLineKind::Error, "usage: /resume <id>");
                return Ok(false);
            };
            match load_session(id) {
                Ok(session) => {
                    state.lines.clear();
                    state.pending_images.clear();
                    state.last_request = None;
                    state.push_line(ChatLineKind::Status, format!("resumed session {}", session.id));
                    state.session = Some(session);
                }
                Err(err) => state.push_line(ChatLineKind::Error, format!("{err:#}")),
            }
        }
        "/image" => {
            let image = unquote_image_input(input.strip_prefix("/image").unwrap_or_default().trim());
            if image.is_empty() {
                state.push_line(
                    ChatLineKind::Error,
                    "usage: /image <URL, local path, Base64, or Data URI>",
                );
                return Ok(false);
            }
            let label = image_label(image);
            state.pending_images.push(image.to_string());
            state.push_line(
                ChatLineKind::Status,
                format!(
                    "image {} queued: {label}; send a message or press Enter to describe it",
                    state.pending_images.len()
                ),
            );
        }
        "/images" => match input.strip_prefix("/images").unwrap_or_default().trim() {
            "clear" => {
                state.pending_images.clear();
                state.push_line(ChatLineKind::Status, "pending images cleared");
            }
            "" => {
                if state.pending_images.is_empty() {
                    state.push_line(ChatLineKind::Status, "no images queued; use /image <input>");
                } else {
                    let labels: Vec<_> = state.pending_images.iter().map(|image| image_label(image)).collect();
                    for (index, label) in labels.into_iter().enumerate() {
                        state.push_line(ChatLineKind::Status, format!("image {}: {label}", index + 1));
                    }
                }
            }
            _ => state.push_line(ChatLineKind::Error, "usage: /images [clear]"),
        },
        "/skills" => {
            state.push_line(
                ChatLineKind::Status,
                "Skills: use /skill <name> to load one into the agent context.",
            );
            for summary in list_skill_summaries().into_iter().take(50) {
                state.push_line(
                    ChatLineKind::Status,
                    format!(
                        "{} - {} ({})",
                        summary.name,
                        summary.description,
                        summary.path.display()
                    ),
                );
            }
        }
        "/skill" => {
            let Some(name) = parts.next() else {
                state.push_line(ChatLineKind::Error, "usage: /skill <name>");
                return Ok(false);
            };
            match load_skill_context(name) {
                Ok(context) => {
                    state.loaded_skills.push(context);
                    state.push_line(ChatLineKind::Status, format!("loaded skill {name}"));
                }
                Err(err) => state.push_line(ChatLineKind::Error, format!("{err:#}")),
            }
        }
        "/tools" => {
            for tool in default_agent_tools() {
                state.push_line(
                    ChatLineKind::Status,
                    format!("{} | {}", tool.name(), tool.description()),
                );
            }
        }
        "/approval" => {
            if parts.next() == Some("toggle") {
                state.toggle_approval_mode();
            }
            state.push_line(
                ChatLineKind::Status,
                format!(
                    "approval: {} (Shift-Tab or /approval toggle)",
                    state.telemetry.approval_label
                ),
            );
        }
        "/compress" => {
            compress_session_messages(state, cfg, true)?;
        }
        "/model" => {
            state.push_line(
                ChatLineKind::Status,
                format!(
                    "text_model={} thinking_text_model={} context={} max_output={}",
                    cfg.text_model,
                    cfg.thinking_text_model.as_deref().unwrap_or("<text_model>"),
                    state.overrides.context_tokens.unwrap_or(cfg.chat_context_tokens),
                    state.overrides.max_output_tokens.unwrap_or(cfg.chat_max_output_tokens)
                ),
            );
        }
        "/thinking" => {
            let current = state.overrides.thinking.unwrap_or(cfg.chat_thinking);
            apply_thinking_setting(state, cfg, !current);
            state.push_line(ChatLineKind::Status, format!("thinking={}", !current));
        }
        "/tasks" => {
            let db = Database::open()?;
            for task in db.list_video_tasks(10)? {
                state.push_line(
                    ChatLineKind::Status,
                    format!(
                        "#{} {} {} {}",
                        task.id,
                        task.phase,
                        task.task_id,
                        task.uri.unwrap_or_default()
                    ),
                );
            }
        }
        _ => {
            state.handle_slash(input);
        }
    }
    Ok(false)
}

pub(crate) fn push_help_lines(state: &mut ChatUiState) {
    for line in help_lines() {
        state.push_line(ChatLineKind::Status, line);
    }
}

fn help_lines() -> Vec<&'static str> {
    vec![
        "Agnes Chat help",
        "Start: type a message to ask for coding work, image generation, async video tasks, asset lookup, or repo changes.",
        "Conversation: /new - Start a new chat. /sessions - List saved chats. /resume <id> - Restore one. /retry - Retry the previous request without adding another user message to session history.",
        "Image understanding: /image <URL, local path, Base64, or Data URI> attaches one image to your next message. Paths may contain spaces. Repeat /image for multiple images. /images lists pending images; /images clear removes them. Press Enter without text to describe queued images.",
        "Skills: /skills - Show available skills with descriptions. /skill <name> - Load a skill into the current agent context.",
        "Tools and media: /tools - List PI and Agnes tools. Async video uses agnes_submit_video so long jobs keep running in the background. /tasks - Show video task progress.",
        "Settings: /model shows model, context, and output limits. /thinking toggles thinking mode. /compress manually compresses session history.",
        "Safety: /approval toggle switches review/auto. Auto mode still requires approval for dangerous commands.",
        "Keys: Tab completes slash commands. Shift-Tab toggles approval mode. Ctrl-R retries the previous request. Ctrl-T expands or folds thinking. Ctrl-E expands or folds tool details. Ctrl-Q quits.",
    ]
}

fn unquote_image_input(input: &str) -> &str {
    for quote in ['\'', '"'] {
        if let Some(inner) = input.strip_prefix(quote).and_then(|value| value.strip_suffix(quote)) {
            return inner;
        }
    }
    input
}

fn image_label(input: &str) -> String {
    use crate::media::input::{ImageInput, classify_input};
    use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

    match classify_input(input) {
        ImageInput::Base64(_) => "Base64 image".into(),
        ImageInput::DataUri(_) => "inline image (Data URI)".into(),
        ImageInput::Url(url) => {
            if let Ok(mut parsed) = url::Url::parse(&url) {
                parsed.set_query(None);
                parsed.set_fragment(None);
                let _ = parsed.set_username("");
                let _ = parsed.set_password(None);
                truncate(parsed.as_str(), 100)
            } else {
                "image URL".into()
            }
        }
        ImageInput::LocalPath(path) => {
            let compact: Vec<u8> = input.bytes().filter(|byte| !byte.is_ascii_whitespace()).collect();
            if let Ok(bytes) = B64.decode(compact)
                && let Ok(mime) = crate::media::jpeg::mime_from_bytes(&bytes)
                && matches!(mime, "image/jpeg" | "image/png" | "image/webp")
            {
                "Base64 image".into()
            } else {
                truncate(&path, 100)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillSummary {
    name: String,
    path: std::path::PathBuf,
    description: String,
}

fn list_skill_summaries() -> Vec<SkillSummary> {
    list_available_skills()
        .into_iter()
        .map(|(name, path)| {
            let description = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| skill_description_from_markdown(&text))
                .unwrap_or_else(|| "No description in SKILL.md".to_string());
            SkillSummary { name, path, description }
        })
        .collect()
}

fn load_skill_context(name: &str) -> Result<String> {
    let (_, path) = list_available_skills()
        .into_iter()
        .find(|(skill_name, _)| skill_name == name)
        .ok_or_else(|| anyhow::anyhow!("skill not found: {name}"))?;
    let text = std::fs::read_to_string(&path)?;
    Ok(format!("## skill: {name}\n{text}"))
}

pub(crate) fn apply_thinking_setting(state: &mut ChatUiState, cfg: &AppConfig, enabled: bool) {
    state.overrides.thinking = Some(enabled);
    state.telemetry.thinking_enabled = enabled;
    state.telemetry.model_label = if enabled {
        cfg.thinking_text_model
            .as_deref()
            .unwrap_or(cfg.text_model.as_str())
            .to_string()
    } else {
        cfg.text_model.clone()
    };
}

pub(crate) fn compress_session_messages(state: &mut ChatUiState, cfg: &AppConfig, force: bool) -> Result<()> {
    use crate::agent::chat::build_system_prompt;
    use crate::agent::context::maybe_compress;

    let Some(session) = state.session.as_mut() else {
        state.push_line(ChatLineKind::Error, "no active session");
        return Ok(());
    };
    let system_prompt = build_system_prompt(&state.loaded_skills)?;
    let mut messages = session.messages.clone();
    let Some(result) = maybe_compress(&mut messages, state.telemetry.context_tokens, &system_prompt, force) else {
        let message = if force {
            "not enough history to compress further"
        } else {
            "context below 90% threshold — no compression needed"
        };
        state.push_line(ChatLineKind::Status, message);
        return Ok(());
    };
    session.replace_messages(messages);
    save_session(session)?;
    state.telemetry.estimated_session_tokens = result.tokens_after;
    state.push_line(
        ChatLineKind::Status,
        format!(
            "compressed {} messages (~{} → ~{} tokens est.)",
            result.removed_count,
            format_u64_tokens(result.tokens_before),
            format_u64_tokens(result.tokens_after)
        ),
    );
    let _ = cfg;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::agent::chat::ChatOverrides;
    use crate::config::AppConfig;

    use super::*;
    use crate::ui::chat::input::InputState;
    use crate::ui::chat::model::ChatUiOptions;

    #[test]
    fn slash_help_adds_status_line() {
        let mut state = ChatUiState::default();

        state.handle_slash("/help");

        assert!(state.lines.iter().any(|line| line.text.contains("/thinking")));
        assert!(state.lines.iter().any(|line| line.text.contains("Start a new chat")));
        assert!(state.lines.iter().any(|line| line.text.contains("Load a skill")));
        assert!(state.lines.iter().any(|line| line.text.contains("Async video")));
        assert!(state.lines.iter().any(|line| line.text.contains("/approval")));
    }

    #[test]
    fn slash_skills_show_skill_descriptions() {
        let mut state = ChatUiState::default();
        let options = ChatUiOptions {
            resume: None,
            images: Vec::new(),
            auto: false,
            max_turns: 4,
            overrides: ChatOverrides::default(),
        };

        handle_slash_command(&mut state, &AppConfig::default(), &options, "/skills").unwrap();

        assert!(
            state
                .lines
                .iter()
                .any(|line| line.text.contains("agnes-aigc-gen") && line.text.contains("image understanding"))
        );
    }

    #[test]
    fn tab_completes_slash_command() {
        let mut state =
            ChatUiState { input: InputState { buffer: "/thi".into(), ..Default::default() }, ..ChatUiState::default() };

        state.complete_command();

        assert_eq!(state.input.buffer, "/thinking ");
    }

    #[test]
    fn tab_completes_skill_name() {
        let mut state = ChatUiState {
            input: InputState { buffer: "/skill agnes".into(), ..Default::default() },
            ..ChatUiState::default()
        };

        state.complete_command();

        assert_eq!(state.input.buffer, "/skill agnes-aigc-gen ");
    }

    fn options() -> ChatUiOptions {
        ChatUiOptions {
            resume: None,
            images: Vec::new(),
            auto: false,
            max_turns: 4,
            overrides: ChatOverrides::default(),
        }
    }

    #[test]
    fn image_attachment_commands_preserve_paths_and_inline_images_without_displaying_bytes() {
        let mut state = ChatUiState::default();
        let cfg = AppConfig::default();
        let inline = format!("data:image/png;base64,{}", "ZGVm".repeat(100));
        for command in [
            "/image \"./my photo, version 2.png\"".to_string(),
            format!("/image {inline}"),
            "/image https://example.com/a.png?token=private#secret".into(),
        ] {
            handle_slash_command(&mut state, &cfg, &options(), &command).unwrap();
        }
        assert_eq!(state.pending_images[0], "./my photo, version 2.png");
        assert_eq!(state.pending_images[1], inline);
        assert_eq!(state.pending_images.len(), 3);
        handle_slash_command(&mut state, &cfg, &options(), "/images").unwrap();
        assert!(
            state
                .lines
                .iter()
                .any(|line| line.text.contains("inline image (Data URI)"))
        );
        assert!(!state.lines.iter().any(|line| line.text.contains("ZGVmZGVm")));
        assert!(!state.lines.iter().any(|line| line.text.contains("token=private")));
        assert!(!state.lines.iter().any(|line| line.text.contains("#secret")));
    }

    #[test]
    fn image_labels_hide_short_and_line_wrapped_base64_images() {
        use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

        for format in [image::ImageFormat::WebP, image::ImageFormat::Png] {
            let mut bytes = Vec::new();
            image::DynamicImage::new_rgb8(1, 1)
                .write_to(&mut std::io::Cursor::new(&mut bytes), format)
                .unwrap();
            let payload = B64.encode(bytes);
            if format == image::ImageFormat::WebP {
                assert!(payload.len() <= 64);
            }
            let wrapped = payload
                .as_bytes()
                .chunks(16)
                .map(|part| std::str::from_utf8(part).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(image_label(&payload), "Base64 image");
            assert_eq!(image_label(&wrapped), "Base64 image");
        }
        assert_eq!(image_label("./photos/my image.png"), "./photos/my image.png");
    }

    #[test]
    fn image_commands_clear_pending_and_new_session_forgets_retry_images() {
        let mut state = ChatUiState::default();
        let cfg = AppConfig::default();
        handle_slash_command(&mut state, &cfg, &options(), "/image ./photo.png").unwrap();
        handle_slash_command(&mut state, &cfg, &options(), "/images clear").unwrap();
        assert!(state.pending_images.is_empty());
        handle_slash_command(&mut state, &cfg, &options(), "/image").unwrap();
        assert!(state.pending_images.is_empty());
        assert_eq!(state.lines.last().unwrap().kind, ChatLineKind::Error);
        handle_slash_command(&mut state, &cfg, &options(), "/image ./other.png").unwrap();
        state.last_request = Some(crate::ui::chat::model::RetryRequest {
            input: "old request".into(),
            history: vec![pi_ai::Message::user_text("old request")],
        });
        handle_slash_command(&mut state, &cfg, &options(), "/new").unwrap();
        assert!(state.pending_images.is_empty());
        assert!(state.last_request.is_none());
    }

    #[test]
    fn session_switch_during_run_keeps_images_in_the_current_session() {
        let session = ChatSession::new("agnes-3.0-flash", true);
        let id = session.id.clone();
        let mut state = ChatUiState {
            busy: true,
            session: Some(session),
            pending_images: vec!["./photo.png".into()],
            ..Default::default()
        };
        for command in ["/new", "/resume another-chat"] {
            handle_slash_command(&mut state, &AppConfig::default(), &options(), command).unwrap();
            assert_eq!(state.session.as_ref().unwrap().id, id);
            assert_eq!(state.pending_images, vec!["./photo.png"]);
        }
    }
}
