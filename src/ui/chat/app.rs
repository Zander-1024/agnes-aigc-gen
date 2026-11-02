use std::io;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use crossterm::ExecutableCommand;
use crossterm::cursor::Hide;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use pi_agent::{AgentEvent, PermissionDecision};
use pi_ai::Message;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;

use crate::agent::approval::ApprovalMode;
use crate::agent::chat::{build_agent_config, build_system_prompt};
use crate::agent::context::{estimate_tokens, maybe_compress, should_auto_compress};
use crate::agent::runner::run_agnes_agent_with_history;
use crate::agent::session::{ChatSession, load_session, save_session};
use crate::agent::vision::build_user_message;
use crate::config::AppConfig;

use super::approval::UiPermission;
use super::commands::{command_hint_for_input, compress_session_messages, handle_slash_command};
use super::input::InputState;
use super::model::{ChatLineKind, ChatUiOptions, RetryRequest, RunPhase, UiEvent};
use super::render::render;
use super::scroll::ScrollView;
use super::state::ChatUiState;
use super::status::{activity_status, approval_label_for, format_u64_tokens, truncate};
use super::telemetry::SessionTelemetry;

pub fn run(options: ChatUiOptions) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?.execute(Hide)?;
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal, &runtime, options);
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    ratatui::restore();
    result
}

fn run_app(terminal: &mut DefaultTerminal, runtime: &tokio::runtime::Runtime, options: ChatUiOptions) -> Result<()> {
    let cfg = AppConfig::load()?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let thinking_enabled = options.overrides.thinking.unwrap_or(cfg.chat_thinking);
    let model_label = if thinking_enabled {
        cfg.thinking_text_model
            .as_deref()
            .unwrap_or(cfg.text_model.as_str())
            .to_string()
    } else {
        cfg.text_model.clone()
    };
    let mut state = ChatUiState {
        status: "Ready".into(),
        session: load_initial_session(&cfg, &options),
        overrides: options.overrides.clone(),
        telemetry: SessionTelemetry {
            model_label,
            directory_label: std::env::current_dir()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| ".".into()),
            thinking_enabled,
            context_tokens: options.overrides.context_tokens.unwrap_or(cfg.chat_context_tokens),
            max_output_tokens: options
                .overrides
                .max_output_tokens
                .unwrap_or(cfg.chat_max_output_tokens),
            approval_label: approval_label_for(options.auto),
            ..Default::default()
        },
        transcript: ScrollView { pinned_to_bottom: true, ..Default::default() },
        input: InputState { blink_at: Instant::now(), ..Default::default() },
        auto_mode: options.auto,
        pending_images: options.images.clone(),
        ..ChatUiState::default()
    };
    if let Some(session) = &state.session {
        state.status = format!("Session {} | model {}", session.id, session.model);
    }
    let _ = ensure_ui_permission(&mut state, tx.clone())?;

    loop {
        while let Ok(event) = rx.try_recv() {
            handle_ui_event(&mut state, event)?;
        }
        state.tick_input_blink();
        terminal.draw(|frame| render(frame, &mut state))?;

        if event::poll(std::time::Duration::from_millis(80))?
            && let Event::Key(key) = event::read()?
            && handle_key(key.code, key.modifiers, &mut state, &cfg, &options, tx.clone(), runtime)?
        {
            break;
        }
    }
    Ok(())
}

fn load_initial_session(cfg: &AppConfig, options: &ChatUiOptions) -> Option<ChatSession> {
    match options.resume.as_deref() {
        Some(id) => load_session(id).ok(),
        None => Some(ChatSession::new(
            cfg.thinking_text_model.as_deref().unwrap_or(cfg.text_model.as_str()),
            options.overrides.thinking.unwrap_or(cfg.chat_thinking),
        )),
    }
}

fn handle_key(
    key: KeyCode,
    mods: KeyModifiers,
    state: &mut ChatUiState,
    cfg: &AppConfig,
    options: &ChatUiOptions,
    tx: mpsc::UnboundedSender<UiEvent>,
    runtime: &tokio::runtime::Runtime,
) -> Result<bool> {
    if key == KeyCode::Char('q') && mods.contains(KeyModifiers::CONTROL) {
        return Ok(true);
    }
    if key == KeyCode::Char('t') && mods.contains(KeyModifiers::CONTROL) {
        state.toggle_thinking();
        return Ok(false);
    }
    if key == KeyCode::Char('e') && mods.contains(KeyModifiers::CONTROL) {
        state.toggle_tools();
        return Ok(false);
    }
    if key == KeyCode::BackTab && !state.approval.is_active() {
        state.toggle_approval_mode();
        return Ok(false);
    }
    if key == KeyCode::Tab && mods.contains(KeyModifiers::CONTROL) && !state.approval.is_active() {
        state.toggle_approval_mode();
        return Ok(false);
    }
    if state.approval.is_active() {
        match key {
            KeyCode::Left | KeyCode::Char('h') => state.approval_choice_left(),
            KeyCode::Right | KeyCode::Char('l') => state.approval_choice_right(),
            KeyCode::Enter => {
                if let Some(decision) = state.confirm_approval_choice() {
                    answer_approval(state, decision);
                }
            }
            KeyCode::Char('y') => answer_approval(state, PermissionDecision::Allow),
            KeyCode::Char('a') => answer_approval(state, PermissionDecision::AllowSession),
            KeyCode::Char('n') | KeyCode::Esc => {
                answer_approval(state, PermissionDecision::Deny { reason: "user denied".into() })
            }
            KeyCode::Up => state.approval_scroll_up(1),
            KeyCode::Down => state.approval_scroll_down(1),
            KeyCode::PageUp => state.approval_scroll_up(state.approval.viewport.max(1) as usize),
            KeyCode::PageDown => state.approval_scroll_down(state.approval.viewport.max(1) as usize),
            _ => {}
        }
        return Ok(false);
    }
    if matches!(key, KeyCode::PageUp | KeyCode::PageDown)
        || (mods.contains(KeyModifiers::CONTROL) && matches!(key, KeyCode::Up | KeyCode::Down | KeyCode::End))
    {
        state.handle_scroll_key(key, mods);
        return Ok(false);
    }
    if key == KeyCode::Left && !mods.contains(KeyModifiers::CONTROL) {
        state.input_cursor_left();
        return Ok(false);
    }
    if key == KeyCode::Right && !mods.contains(KeyModifiers::CONTROL) {
        state.input_cursor_right();
        return Ok(false);
    }
    if key == KeyCode::Home && !mods.contains(KeyModifiers::CONTROL) {
        state.set_input_cursor(0);
        return Ok(false);
    }
    if key == KeyCode::End && !mods.contains(KeyModifiers::CONTROL) {
        state.sync_input_cursor_end();
        return Ok(false);
    }
    if key == KeyCode::Up
        && state.input.cursor == 0
        && state.input.buffer.is_empty()
        && !mods.contains(KeyModifiers::CONTROL)
    {
        state.input_history_prev();
        return Ok(false);
    }
    if key == KeyCode::Down && state.input.history_cursor.is_some() && !mods.contains(KeyModifiers::CONTROL) {
        state.input_history_next();
        return Ok(false);
    }
    if key == KeyCode::Char('r') && mods.contains(KeyModifiers::CONTROL) {
        retry_last_request(state, cfg, options, tx, runtime)?;
        return Ok(false);
    }

    match key {
        KeyCode::Esc => return Ok(false),
        KeyCode::Tab => {
            state.complete_command();
        }
        KeyCode::Backspace => {
            state.reset_input_history_navigation();
            state.input_backspace();
            state.input.completion_hint = None;
        }
        KeyCode::Delete => {
            state.reset_input_history_navigation();
            state.input_delete();
            state.input.completion_hint = None;
        }
        KeyCode::Enter => {
            let input = state.input.buffer.trim().to_string();
            state.record_input_history(&input);
            state.input.buffer.clear();
            state.set_input_cursor(0);
            state.reset_input_history_navigation();
            state.input.completion_hint = None;
            if input.is_empty() && state.pending_images.is_empty() {
                return Ok(false);
            }
            if input == "/retry" {
                retry_last_request(state, cfg, options, tx, runtime)?;
                return Ok(false);
            }
            if input.starts_with('/') {
                return handle_slash_command(state, cfg, options, &input);
            }
            submit_prompt(state, cfg, options, input, tx, runtime)?;
        }
        KeyCode::Char(c) if !c.is_control() => {
            state.reset_input_history_navigation();
            state.input_insert(c);
            state.input.completion_hint = command_hint_for_input(&state.input.buffer);
        }
        _ => {}
    }
    Ok(false)
}

fn answer_approval(state: &mut ChatUiState, decision: PermissionDecision) {
    if let Some(request) = state.approval.pending.take() {
        let label = match &decision {
            PermissionDecision::Allow => "approved",
            PermissionDecision::AllowSession => "approved for session",
            PermissionDecision::Deny { .. } => "denied",
        };
        state.push_line(ChatLineKind::Status, format!("{} {label}", request.tool_name));
        let _ = request.respond_to.send(decision);
    }
}

fn submit_prompt(
    state: &mut ChatUiState,
    cfg: &AppConfig,
    options: &ChatUiOptions,
    input: String,
    tx: mpsc::UnboundedSender<UiEvent>,
    runtime: &tokio::runtime::Runtime,
) -> Result<()> {
    if state.busy {
        state.status = activity_status(state);
        return Ok(());
    }
    let message = match build_user_message(&input, &state.pending_images) {
        Ok(message) => message,
        Err(err) => {
            state.push_line(ChatLineKind::Error, format!("{err:#}"));
            state.input.buffer = input;
            state.sync_input_cursor_end();
            return Ok(());
        }
    };
    let agent = match build_ui_agent(state, cfg, options, tx.clone()) {
        Ok(agent) => agent,
        Err(err) => {
            state.push_line(ChatLineKind::Error, format!("{err:#}"));
            state.input.buffer = input;
            state.sync_input_cursor_end();
            return Ok(());
        }
    };
    let mut session = state
        .session
        .clone()
        .unwrap_or_else(|| ChatSession::new(&agent.runtime.model.id, agent.runtime.enable_thinking));
    session.model = agent.runtime.model.id.clone();
    session.thinking = agent.runtime.enable_thinking;
    let mut history = session.messages.clone();
    history.push(message);
    if let Err(err) = prepare_agent_history(state, &mut history) {
        state.push_line(ChatLineKind::Error, format!("{err:#}"));
        state.input.buffer = input;
        state.sync_input_cursor_end();
        return Ok(());
    }
    let display = user_message_label(&input, state.pending_images.len());
    state.push_user(&display);
    state.pending_images.clear();
    state.last_request = Some(RetryRequest { input: display, history: history.clone() });
    start_agent_run(state, agent, session, history, tx, runtime, "Running agent...")
}

fn user_message_label(input: &str, image_count: usize) -> String {
    if image_count == 0 {
        input.to_string()
    } else if input.is_empty() {
        format!("[{image_count} image(s)]")
    } else {
        format!("{input}\n[{image_count} image(s)]")
    }
}

fn prepare_agent_history(state: &mut ChatUiState, history: &mut Vec<Message>) -> Result<()> {
    let system_prompt = build_system_prompt(&state.loaded_skills)?;
    let used = estimate_tokens(history, &system_prompt);
    state.telemetry.estimated_session_tokens = used;
    if let Some(result) = maybe_compress(history, state.telemetry.context_tokens, &system_prompt, false) {
        state.push_line(
            ChatLineKind::Status,
            format!(
                "auto-compressed {} messages (~{} → ~{} tokens est.)",
                result.removed_count,
                format_u64_tokens(result.tokens_before),
                format_u64_tokens(result.tokens_after)
            ),
        );
        state.telemetry.estimated_session_tokens = result.tokens_after;
    }
    Ok(())
}

fn maybe_auto_compress_session_after_run(state: &mut ChatUiState, cfg: &AppConfig) -> Result<()> {
    let used = state
        .telemetry
        .last_prompt_tokens
        .max(state.telemetry.estimated_session_tokens);
    if should_auto_compress(used, state.telemetry.context_tokens) {
        compress_session_messages(state, cfg, false)?;
    }
    Ok(())
}

fn retry_last_request(
    state: &mut ChatUiState,
    cfg: &AppConfig,
    options: &ChatUiOptions,
    tx: mpsc::UnboundedSender<UiEvent>,
    runtime: &tokio::runtime::Runtime,
) -> Result<()> {
    if state.busy {
        state.status = activity_status(state);
        return Ok(());
    }
    let Some(retry) = state.last_request_for_retry() else {
        state.push_line(ChatLineKind::Error, "no previous request to retry");
        return Ok(());
    };
    let agent = match build_ui_agent(state, cfg, options, tx.clone()) {
        Ok(agent) => agent,
        Err(err) => {
            state.push_line(ChatLineKind::Error, format!("{err:#}"));
            return Ok(());
        }
    };
    let mut session = state
        .session
        .take()
        .unwrap_or_else(|| ChatSession::new(&agent.runtime.model.id, agent.runtime.enable_thinking));
    session.model = agent.runtime.model.id.clone();
    session.thinking = agent.runtime.enable_thinking;
    state.push_line(
        ChatLineKind::Status,
        format!("retrying previous request: {}", truncate(&retry.input, 80)),
    );
    start_agent_run(
        state,
        agent,
        session,
        retry.history,
        tx,
        runtime,
        "Retrying previous request...",
    )
}

fn build_ui_agent(
    state: &mut ChatUiState,
    cfg: &AppConfig,
    options: &ChatUiOptions,
    tx: mpsc::UnboundedSender<UiEvent>,
) -> Result<crate::agent::runner::AgnesAgentConfig> {
    let permission = ensure_ui_permission(state, tx)?;
    build_agent_config(
        cfg,
        state.overrides.clone(),
        options.max_turns,
        permission,
        &state.loaded_skills,
    )
}

fn ensure_ui_permission(state: &mut ChatUiState, tx: mpsc::UnboundedSender<UiEvent>) -> Result<Arc<UiPermission>> {
    if let Some(permission) = &state.permission {
        return Ok(permission.clone());
    }
    let workspace = std::env::current_dir()?;
    let mode = if state.auto_mode {
        ApprovalMode::Auto
    } else {
        ApprovalMode::Review
    };
    let permission = Arc::new(UiPermission::new(workspace, mode, tx));
    state.permission = Some(permission.clone());
    Ok(permission)
}

fn start_agent_run(
    state: &mut ChatUiState,
    mut agent: crate::agent::runner::AgnesAgentConfig,
    session: ChatSession,
    history: Vec<Message>,
    tx: mpsc::UnboundedSender<UiEvent>,
    runtime: &tokio::runtime::Runtime,
    status: &str,
) -> Result<()> {
    agent.runtime.session_id = Some(session.id.clone());
    state.session = Some(session);
    state.busy = true;
    state.run_phase = RunPhase::Waiting;
    state.status = status.into();

    let agent_tx = tx.clone();
    let agent_for_task = agent.clone();
    runtime.spawn(async move {
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let runner =
            tokio::spawn(async move { run_agnes_agent_with_history(&agent_for_task, history, Some(events_tx)).await });
        while let Some(event) = events_rx.recv().await {
            let _ = agent_tx.send(UiEvent::Agent(event));
        }
        let result = match runner.await {
            Ok(Ok(run)) => Ok((run.messages, run.stopped_at_turn_limit)),
            Ok(Err(err)) => Err(format!("{err:#}")),
            Err(err) => Err(format!("agent task failed: {err}")),
        };
        let _ = agent_tx.send(UiEvent::Finished(result));
    });
    Ok(())
}

fn handle_ui_event(state: &mut ChatUiState, event: UiEvent) -> Result<()> {
    match event {
        UiEvent::Agent(event) => match event {
            AgentEvent::AgentStart | AgentEvent::TurnStart => {
                state.run_phase = RunPhase::Waiting;
                state.status = activity_status(state);
            }
            AgentEvent::TextDelta { delta } => {
                state.run_phase = RunPhase::Responding;
                state.status = activity_status(state);
                state.push_assistant_delta(&delta);
            }
            AgentEvent::ThinkingDelta { delta } => {
                if state.run_phase != RunPhase::Responding {
                    state.run_phase = RunPhase::Thinking;
                    state.status = activity_status(state);
                }
                state.push_thinking_delta(&delta);
            }
            AgentEvent::AssistantMessage { message } => state.update_usage_from_assistant(&message),
            AgentEvent::ToolExecutionStart { tool_name, args, .. } => {
                state.run_phase = RunPhase::Tool(tool_name.clone());
                state.status = activity_status(state);
                state.push_line(ChatLineKind::Tool, format!("→ {tool_name}({args})"))
            }
            AgentEvent::ToolExecutionEnd { tool_name, is_error, .. } => {
                state.run_phase = RunPhase::Waiting;
                state.status = activity_status(state);
                state.push_line(
                    ChatLineKind::Tool,
                    format!("← {tool_name} {}", if is_error { "error" } else { "ok" }),
                )
            }
            AgentEvent::PermissionDenied { tool_name, reason } => {
                state.push_line(ChatLineKind::Error, format!("{tool_name} denied: {reason}"));
            }
            AgentEvent::TurnEnd | AgentEvent::AgentEnd { .. } => {
                state.run_phase = RunPhase::Waiting;
                state.status = activity_status(state);
            }
            _ => {}
        },
        UiEvent::Approval(request) => {
            state.begin_approval(request);
        }
        UiEvent::AutoApproved { tool_name } => {
            state.push_line(ChatLineKind::Status, format!("auto-approved {tool_name}"));
        }
        UiEvent::Finished(result) => {
            state.busy = false;
            state.run_phase = RunPhase::Idle;
            match result {
                Ok((messages, stopped_at_turn_limit)) => {
                    if let Some(session) = &mut state.session {
                        session.replace_messages(messages);
                        if let Err(err) = save_session(session) {
                            state.push_line(ChatLineKind::Error, format!("session save failed: {err:#}"));
                        } else {
                            state.status = format!("Saved session {}", session.id);
                        }
                    }
                    if let Ok(cfg) = AppConfig::load() {
                        if let Ok(system_prompt) = build_system_prompt(&state.loaded_skills) {
                            state.telemetry.estimated_session_tokens = estimate_tokens(
                                state.session.as_ref().map(|s| s.messages.as_slice()).unwrap_or(&[]),
                                &system_prompt,
                            );
                        }
                        let _ = maybe_auto_compress_session_after_run(state, &cfg);
                    }
                    if stopped_at_turn_limit {
                        state.push_line(ChatLineKind::Status, "agent stopped at max turn limit");
                    }
                }
                Err(err) => {
                    state.status = "Error - use /retry or Ctrl-R to retry the previous request".into();
                    state.push_line(ChatLineKind::Error, err);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_image_preparation_keeps_pending_images_prompt_and_session() {
        let missing = std::env::temp_dir().join(format!("agnes-missing-image-{}.png", rand::random::<u64>()));
        let image = missing.display().to_string();
        let mut session = ChatSession::new("agnes-3.0-flash", true);
        session.messages.push(Message::user_text("earlier context"));
        let session_id = session.id.clone();
        let mut state =
            ChatUiState { pending_images: vec![image.clone()], session: Some(session), ..Default::default() };
        let options = ChatUiOptions {
            resume: None,
            images: Vec::new(),
            auto: false,
            max_turns: 4,
            overrides: Default::default(),
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (tx, _) = mpsc::unbounded_channel();

        submit_prompt(
            &mut state,
            &AppConfig::default(),
            &options,
            "describe this".into(),
            tx,
            &runtime,
        )
        .unwrap();

        assert_eq!(state.pending_images, vec![image]);
        assert_eq!(state.input.buffer, "describe this");
        assert_eq!(state.session.as_ref().unwrap().id, session_id);
        assert_eq!(state.session.as_ref().unwrap().messages.len(), 1);
        assert!(!state.busy);
        assert!(state.last_request.is_none());
        assert_eq!(state.lines.last().unwrap().kind, ChatLineKind::Error);
    }
}
