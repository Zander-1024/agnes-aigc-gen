use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use crossterm::ExecutableCommand;
use crossterm::cursor::Hide;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use pi_agent::{AgentEvent, PermissionDecision, PermissionPolicy};
use pi_ai::Message;
use ratatui::DefaultTerminal;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::agent::approval::{ApprovalDecision, ApprovalMode, ApprovalPolicy};
use crate::agent::chat::{ChatOverrides, build_agent_config, build_system_prompt};
use crate::agent::context::{estimate_tokens, maybe_compress, should_auto_compress, usage_percent};
use crate::agent::runner::run_agnes_agent_with_history;
use crate::agent::session::{ChatSession, list_sessions, load_session, save_session};
use crate::agent::tools::{default_agent_tools, list_available_skills};
use crate::cli::ChatArgs;
use crate::config::AppConfig;
use crate::db::Database;

#[derive(Debug, Clone)]
pub struct ChatUiOptions {
    pub resume: Option<String>,
    pub auto: bool,
    pub max_turns: u32,
    pub overrides: ChatOverrides,
}

impl From<ChatArgs> for ChatUiOptions {
    fn from(args: ChatArgs) -> Self {
        let overrides = args.overrides();
        Self { resume: args.resume, auto: args.auto, max_turns: args.max_turns, overrides }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatLineKind {
    User,
    Assistant,
    Thinking,
    Tool,
    Status,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    pub kind: ChatLineKind,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum RunPhase {
    #[default]
    Idle,
    Waiting,
    Thinking,
    Responding,
    Tool(String),
}

pub struct ChatUiState {
    pub lines: Vec<ChatLine>,
    input: String,
    status: String,
    session: Option<ChatSession>,
    busy: bool,
    pending_approval: Option<ApprovalRequest>,
    last_request: Option<RetryRequest>,
    loaded_skills: Vec<String>,
    overrides: ChatOverrides,
    thinking_expanded: bool,
    tools_expanded: bool,
    completion_hint: Option<String>,
    model_label: String,
    directory_label: String,
    approval_label: String,
    thinking_enabled: bool,
    context_tokens: u32,
    max_output_tokens: u32,
    scroll_from_bottom: usize,
    pinned_to_bottom: bool,
    transcript_viewport: u16,
    input_history: Vec<String>,
    input_history_cursor: Option<usize>,
    input_history_draft: String,
    auto_mode: bool,
    permission: Option<Arc<UiPermission>>,
    last_prompt_tokens: u64,
    last_cache_read: u64,
    estimated_session_tokens: u64,
    approval_choice: usize,
    approval_scroll_from_bottom: usize,
    approval_viewport: u16,
    run_phase: RunPhase,
    input_cursor: usize,
    input_blink_on: bool,
    input_blink_at: Instant,
}

const APPROVAL_CHOICE_COUNT: usize = 3;

impl Default for ChatUiState {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            input: String::new(),
            status: String::new(),
            session: None,
            busy: false,
            pending_approval: None,
            last_request: None,
            loaded_skills: Vec::new(),
            overrides: ChatOverrides::default(),
            thinking_expanded: false,
            tools_expanded: false,
            completion_hint: None,
            model_label: String::new(),
            directory_label: String::new(),
            approval_label: String::new(),
            thinking_enabled: false,
            context_tokens: 0,
            max_output_tokens: 0,
            scroll_from_bottom: 0,
            pinned_to_bottom: false,
            transcript_viewport: 0,
            input_history: Vec::new(),
            input_history_cursor: None,
            input_history_draft: String::new(),
            auto_mode: false,
            permission: None,
            last_prompt_tokens: 0,
            last_cache_read: 0,
            estimated_session_tokens: 0,
            approval_choice: 0,
            approval_scroll_from_bottom: 0,
            approval_viewport: 0,
            run_phase: RunPhase::Idle,
            input_cursor: 0,
            input_blink_on: true,
            input_blink_at: Instant::now(),
        }
    }
}

const USER_BUBBLE_BG: Color = Color::Rgb(30, 60, 90);
const USER_MAX_WIDTH_PERCENT: usize = 75;

impl ChatUiState {
    pub fn push_line(&mut self, kind: ChatLineKind, text: impl Into<String>) {
        self.lines.push(ChatLine { kind, text: text.into() });
        self.on_new_content();
    }

    pub fn push_user(&mut self, text: impl Into<String>) {
        self.push_line(ChatLineKind::User, text);
    }

    pub fn push_assistant_delta(&mut self, delta: &str) {
        if let Some(last) = self.lines.last_mut()
            && last.kind == ChatLineKind::Assistant
        {
            last.text.push_str(delta);
            self.on_new_content();
            return;
        }
        self.push_line(ChatLineKind::Assistant, delta);
    }

    pub fn push_thinking_delta(&mut self, delta: &str) {
        if let Some(last) = self.lines.last_mut()
            && last.kind == ChatLineKind::Thinking
        {
            last.text.push_str(delta);
            self.on_new_content();
            return;
        }
        self.push_line(ChatLineKind::Thinking, delta);
    }

    fn on_new_content(&mut self) {
        if self.pinned_to_bottom {
            self.scroll_from_bottom = 0;
        }
    }

    fn scroll_up(&mut self, lines: usize) {
        self.pinned_to_bottom = false;
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_add(lines);
    }

    fn scroll_down(&mut self, lines: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(lines);
        if self.scroll_from_bottom == 0 {
            self.pinned_to_bottom = true;
        }
    }

    fn scroll_to_bottom(&mut self) {
        self.scroll_from_bottom = 0;
        self.pinned_to_bottom = true;
    }

    #[cfg(test)]
    pub(crate) fn scroll_offset(&self) -> usize {
        self.scroll_from_bottom
    }

    fn handle_scroll_key(&mut self, key: KeyCode, mods: KeyModifiers) {
        let page = self.transcript_viewport.max(1) as usize;
        match key {
            KeyCode::PageUp => self.scroll_up(page),
            KeyCode::PageDown => self.scroll_down(page),
            KeyCode::Up if mods.contains(KeyModifiers::CONTROL) => self.scroll_up(1),
            KeyCode::Down if mods.contains(KeyModifiers::CONTROL) => self.scroll_down(1),
            KeyCode::End if mods.contains(KeyModifiers::CONTROL) => self.scroll_to_bottom(),
            KeyCode::End => {}
            _ => {}
        }
    }

    pub(crate) fn record_input_history(&mut self, input: &str) {
        if input.is_empty() {
            return;
        }
        if self.input_history.last().is_some_and(|last| last == input) {
            return;
        }
        self.input_history.push(input.to_string());
    }

    pub(crate) fn input_history_prev(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        if self.input_history_cursor.is_none() {
            self.input_history_draft = self.input.clone();
            self.input_history_cursor = Some(self.input_history.len() - 1);
        } else if let Some(index) = self.input_history_cursor
            && index > 0
        {
            self.input_history_cursor = Some(index - 1);
        }
        if let Some(index) = self.input_history_cursor {
            self.input = self.input_history[index].clone();
            self.sync_input_cursor_end();
            self.completion_hint = command_hint_for_input(&self.input);
        }
    }

    pub(crate) fn input_history_next(&mut self) {
        let Some(index) = self.input_history_cursor else {
            return;
        };
        if index + 1 >= self.input_history.len() {
            self.input_history_cursor = None;
            self.input = self.input_history_draft.clone();
        } else {
            self.input_history_cursor = Some(index + 1);
            self.input = self.input_history[index + 1].clone();
        }
        self.sync_input_cursor_end();
        self.completion_hint = command_hint_for_input(&self.input);
    }

    fn reset_input_history_navigation(&mut self) {
        self.input_history_cursor = None;
        self.input_history_draft.clear();
    }

    fn input_char_count(&self) -> usize {
        self.input.chars().count()
    }

    fn set_input_cursor(&mut self, cursor: usize) {
        self.input_cursor = cursor.min(self.input_char_count());
    }

    fn input_cursor_left(&mut self) {
        self.input_cursor = self.input_cursor.saturating_sub(1);
    }

    fn input_cursor_right(&mut self) {
        if self.input_cursor < self.input_char_count() {
            self.input_cursor += 1;
        }
    }

    fn input_insert(&mut self, ch: char) {
        let before: String = self.input.chars().take(self.input_cursor).collect();
        let after: String = self.input.chars().skip(self.input_cursor).collect();
        self.input = format!("{before}{ch}{after}");
        self.input_cursor += 1;
    }

    fn input_backspace(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let before: String = self.input.chars().take(self.input_cursor.saturating_sub(1)).collect();
        let after: String = self.input.chars().skip(self.input_cursor).collect();
        self.input = format!("{before}{after}");
        self.input_cursor -= 1;
    }

    fn input_delete(&mut self) {
        if self.input_cursor >= self.input_char_count() {
            return;
        }
        let before: String = self.input.chars().take(self.input_cursor).collect();
        let after: String = self.input.chars().skip(self.input_cursor + 1).collect();
        self.input = format!("{before}{after}");
    }

    fn sync_input_cursor_end(&mut self) {
        self.input_cursor = self.input_char_count();
    }

    fn tick_input_blink(&mut self) {
        if self.input_blink_at.elapsed() >= Duration::from_millis(530) {
            self.input_blink_on = !self.input_blink_on;
            self.input_blink_at = Instant::now();
        }
    }

    fn toggle_approval_mode(&mut self) {
        self.auto_mode = !self.auto_mode;
        self.approval_label = approval_label_for(self.auto_mode);
        if let Some(permission) = &self.permission {
            permission.set_mode(if self.auto_mode {
                ApprovalMode::Auto
            } else {
                ApprovalMode::Review
            });
        }
        self.status = format!("Approval mode: {}", self.approval_label);
    }

    fn begin_approval(&mut self, request: ApprovalRequest) {
        self.approval_choice = 0;
        self.approval_scroll_from_bottom = 0;
        self.status = format!("Approval required: {}", request.tool_name);
        self.pending_approval = Some(request);
    }

    pub(crate) fn approval_choice_left(&mut self) {
        self.approval_choice = (self.approval_choice + APPROVAL_CHOICE_COUNT - 1) % APPROVAL_CHOICE_COUNT;
    }

    pub(crate) fn approval_choice_right(&mut self) {
        self.approval_choice = (self.approval_choice + 1) % APPROVAL_CHOICE_COUNT;
    }

    #[cfg(test)]
    pub(crate) fn approval_choice_index(&self) -> usize {
        self.approval_choice
    }

    fn approval_scroll_up(&mut self, lines: usize) {
        self.approval_scroll_from_bottom = self.approval_scroll_from_bottom.saturating_add(lines);
    }

    fn approval_scroll_down(&mut self, lines: usize) {
        self.approval_scroll_from_bottom = self.approval_scroll_from_bottom.saturating_sub(lines);
    }

    fn confirm_approval_choice(&mut self) -> Option<PermissionDecision> {
        match self.approval_choice {
            0 => Some(PermissionDecision::Allow),
            1 => Some(PermissionDecision::AllowSession),
            _ => Some(PermissionDecision::Deny { reason: "user denied".into() }),
        }
    }

    fn update_usage_from_assistant(&mut self, message: &Message) {
        if let Message::Assistant(assistant) = message {
            self.last_prompt_tokens = assistant.usage.input;
            self.last_cache_read = assistant.usage.cache_read;
        }
    }

    pub fn handle_slash(&mut self, slash: &str) -> bool {
        match slash.split_whitespace().next().unwrap_or("") {
            "/help" => {
                push_help_lines(self);
                true
            }
            "/quit" | "/exit" => false,
            _ => {
                self.push_line(ChatLineKind::Error, format!("unknown command: {slash}"));
                true
            }
        }
    }

    pub fn toggle_thinking(&mut self) {
        self.thinking_expanded = !self.thinking_expanded;
        self.status = format!(
            "Thinking {}",
            if self.thinking_expanded { "expanded" } else { "folded" }
        );
    }

    pub fn toggle_tools(&mut self) {
        self.tools_expanded = !self.tools_expanded;
        self.status = format!(
            "Tool details {}",
            if self.tools_expanded { "expanded" } else { "folded" }
        );
    }

    fn last_request_for_retry(&self) -> Option<RetryRequest> {
        self.last_request.clone()
    }

    pub fn complete_command(&mut self) {
        if !self.input.starts_with('/') {
            return;
        }
        if let Some(prefix) = self.input.strip_prefix("/skill ").map(str::to_string) {
            let matches = skill_name_matches(prefix.trim());
            match matches.as_slice() {
                [only] => {
                    self.input = format!("/skill {only} ");
                    self.sync_input_cursor_end();
                    self.completion_hint = None;
                }
                [] => {
                    self.completion_hint = Some("No matching skill".into());
                }
                many => {
                    self.completion_hint = Some(format!("Skills: {}", many.join("  ")));
                }
            }
            return;
        }
        let prefix = self.input.split_whitespace().next().unwrap_or("");
        let matches: Vec<&str> = SLASH_COMMANDS
            .iter()
            .copied()
            .filter(|command| command.starts_with(prefix))
            .collect();
        match matches.as_slice() {
            [only] => {
                self.input = format!("{only} ");
                self.sync_input_cursor_end();
                self.completion_hint = None;
            }
            [] => {
                self.completion_hint = Some("No matching command".into());
            }
            many => {
                self.completion_hint = Some(format!("Completions: {}", many.join("  ")));
            }
        }
    }
}

const SLASH_COMMANDS: &[&str] = &[
    "/help",
    "/new",
    "/sessions",
    "/resume",
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

#[derive(Debug, Clone)]
struct RetryRequest {
    input: String,
    history: Vec<Message>,
}

struct ApprovalRequest {
    tool_name: String,
    args: Value,
    respond_to: oneshot::Sender<PermissionDecision>,
}

enum UiEvent {
    Agent(AgentEvent),
    Approval(ApprovalRequest),
    AutoApproved { tool_name: String },
    Finished(std::result::Result<(Vec<Message>, bool), String>),
}

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
        pinned_to_bottom: true,
        auto_mode: options.auto,
        approval_label: approval_label_for(options.auto),
        input_blink_at: Instant::now(),
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
    if key == KeyCode::BackTab && state.pending_approval.is_none() {
        state.toggle_approval_mode();
        return Ok(false);
    }
    if key == KeyCode::Tab && mods.contains(KeyModifiers::CONTROL) && state.pending_approval.is_none() {
        state.toggle_approval_mode();
        return Ok(false);
    }
    if state.pending_approval.is_some() {
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
            KeyCode::PageUp => state.approval_scroll_up(state.approval_viewport.max(1) as usize),
            KeyCode::PageDown => state.approval_scroll_down(state.approval_viewport.max(1) as usize),
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
    if key == KeyCode::Up && state.input_cursor == 0 && state.input.is_empty() && !mods.contains(KeyModifiers::CONTROL)
    {
        state.input_history_prev();
        return Ok(false);
    }
    if key == KeyCode::Down && state.input_history_cursor.is_some() && !mods.contains(KeyModifiers::CONTROL) {
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
            state.completion_hint = None;
        }
        KeyCode::Delete => {
            state.reset_input_history_navigation();
            state.input_delete();
            state.completion_hint = None;
        }
        KeyCode::Enter => {
            let input = state.input.trim().to_string();
            state.record_input_history(&input);
            state.input.clear();
            state.set_input_cursor(0);
            state.reset_input_history_navigation();
            state.completion_hint = None;
            if input.is_empty() {
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
            state.completion_hint = command_hint_for_input(&state.input);
        }
        _ => {}
    }
    Ok(false)
}

fn answer_approval(state: &mut ChatUiState, decision: PermissionDecision) {
    if let Some(request) = state.pending_approval.take() {
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
    state.push_user(&input);
    let mut history = session.messages.clone();
    history.push(Message::user_text(input.clone()));
    prepare_agent_history(state, &mut history)?;
    state.last_request = Some(RetryRequest { input, history: history.clone() });
    start_agent_run(state, agent, session, history, tx, runtime, "Running agent...")
}

fn prepare_agent_history(state: &mut ChatUiState, history: &mut Vec<Message>) -> Result<()> {
    let system_prompt = build_system_prompt(&state.loaded_skills)?;
    let used = estimate_tokens(history, &system_prompt);
    state.estimated_session_tokens = used;
    if let Some(result) = maybe_compress(history, state.context_tokens, &system_prompt, false) {
        state.push_line(
            ChatLineKind::Status,
            format!(
                "auto-compressed {} messages (~{} → ~{} tokens est.)",
                result.removed_count,
                format_u64_tokens(result.tokens_before),
                format_u64_tokens(result.tokens_after)
            ),
        );
        state.estimated_session_tokens = result.tokens_after;
    }
    Ok(())
}

fn compress_session_messages(state: &mut ChatUiState, cfg: &AppConfig, force: bool) -> Result<()> {
    let Some(session) = state.session.as_mut() else {
        state.push_line(ChatLineKind::Error, "no active session");
        return Ok(());
    };
    let system_prompt = build_system_prompt(&state.loaded_skills)?;
    let mut messages = session.messages.clone();
    let Some(result) = maybe_compress(&mut messages, state.context_tokens, &system_prompt, force) else {
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
    state.estimated_session_tokens = result.tokens_after;
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

fn maybe_auto_compress_session_after_run(state: &mut ChatUiState, cfg: &AppConfig) -> Result<()> {
    let used = state.last_prompt_tokens.max(state.estimated_session_tokens);
    if should_auto_compress(used, state.context_tokens) {
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
    agent: crate::agent::runner::AgnesAgentConfig,
    session: ChatSession,
    history: Vec<Message>,
    tx: mpsc::UnboundedSender<UiEvent>,
    runtime: &tokio::runtime::Runtime,
    status: &str,
) -> Result<()> {
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
                            state.estimated_session_tokens = estimate_tokens(
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

fn command_hint_for_input(input: &str) -> Option<String> {
    if !input.starts_with('/') {
        return None;
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

fn skill_name_matches(prefix: &str) -> Vec<String> {
    list_available_skills()
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| name.starts_with(prefix))
        .collect()
}

fn handle_slash_command(
    state: &mut ChatUiState,
    cfg: &AppConfig,
    _options: &ChatUiOptions,
    input: &str,
) -> Result<bool> {
    let mut parts = input.split_whitespace();
    let cmd = parts.next().unwrap_or("");
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
                    state.push_line(ChatLineKind::Status, format!("resumed session {}", session.id));
                    state.session = Some(session);
                }
                Err(err) => state.push_line(ChatLineKind::Error, format!("{err:#}")),
            }
        }
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
                format!("approval: {} (Shift-Tab or /approval toggle)", state.approval_label),
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

fn push_help_lines(state: &mut ChatUiState) {
    for line in help_lines() {
        state.push_line(ChatLineKind::Status, line);
    }
}

fn help_lines() -> Vec<&'static str> {
    vec![
        "Agnes Chat help",
        "Start: type a message to ask for coding work, image generation, async video tasks, asset lookup, or repo changes.",
        "Conversation: /new - Start a new chat. /sessions - List saved chats. /resume <id> - Restore one. /retry - Retry the previous request without adding another user message to session history.",
        "Skills: /skills - Show available skills with descriptions. /skill <name> - Load a skill into the current agent context.",
        "Tools and media: /tools - List PI and Agnes tools. Async video uses agnes_submit_video so long jobs keep running in the background. /tasks - Show video task progress.",
        "Settings: /model shows model, context, and output limits. /thinking toggles thinking mode. /compress manually compresses session history.",
        "Safety: /approval toggle switches review/auto. Auto mode still requires approval for dangerous commands.",
        "Keys: Tab completes slash commands. Shift-Tab toggles approval mode. Ctrl-R retries the previous request. Ctrl-T expands or folds thinking. Ctrl-E expands or folds tool details. Ctrl-Q quits.",
    ]
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

fn skill_description_from_markdown(text: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()? != "---" {
        return first_body_line(text);
    }
    while let Some(line) = lines.next() {
        if line == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix("description:") {
            let value = value.trim();
            if matches!(value, ">-" | ">" | "|-" | "|") {
                let folded = collect_folded_yaml_value(&mut lines);
                if !folded.is_empty() {
                    return Some(folded);
                }
            } else if let Some(description) = yaml_scalar(value) {
                return Some(description);
            }
        }
    }
    let body = lines.collect::<Vec<_>>().join("\n");
    first_body_line(&body)
}

fn collect_folded_yaml_value<'a>(lines: &mut impl Iterator<Item = &'a str>) -> String {
    let mut parts = Vec::new();
    for line in lines.by_ref() {
        if line == "---" {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            parts.push(line.trim());
        } else {
            break;
        }
    }
    parts.join(" ")
}

fn yaml_scalar(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('"').trim_matches('\'').trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn first_body_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && *line != "---")
        .map(|line| line.to_string())
}

fn load_skill_context(name: &str) -> Result<String> {
    let (_, path) = list_available_skills()
        .into_iter()
        .find(|(skill_name, _)| skill_name == name)
        .ok_or_else(|| anyhow::anyhow!("skill not found: {name}"))?;
    let text = std::fs::read_to_string(&path)?;
    Ok(format!("## skill: {name}\n{text}"))
}

fn apply_thinking_setting(state: &mut ChatUiState, cfg: &AppConfig, enabled: bool) {
    state.overrides.thinking = Some(enabled);
    state.thinking_enabled = enabled;
    state.model_label = if enabled {
        cfg.thinking_text_model
            .as_deref()
            .unwrap_or(cfg.text_model.as_str())
            .to_string()
    } else {
        cfg.text_model.clone()
    };
}

fn render(frame: &mut ratatui::Frame, state: &mut ChatUiState) {
    let approval_active = state.pending_approval.is_some();
    let completion_height = if state.completion_hint.is_some() { 2 } else { 0 };
    let mut constraints = vec![Constraint::Length(9), Constraint::Min(3)];
    if approval_active {
        constraints.push(Constraint::Percentage(32));
    }
    if completion_height > 0 {
        constraints.push(Constraint::Length(completion_height));
    }
    constraints.push(Constraint::Length(3));
    constraints.push(Constraint::Length(1));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(frame.area());

    let mut idx = 0;
    render_header(frame, chunks[idx], state);
    idx += 1;
    render_transcript(frame, chunks[idx], state);
    idx += 1;
    if approval_active {
        render_approval(frame, chunks[idx], state);
        idx += 1;
    }
    if completion_height > 0 {
        render_completion(frame, chunks[idx], state);
        idx += 1;
    }
    let input_block = Block::default().title(input_title(state)).borders(Borders::TOP);
    let input_inner = input_block.inner(chunks[idx]);
    let prompt = "> ";
    let max_width = input_inner.width.max(1) as usize;
    let (display, cursor_col) = input_viewport(prompt, &state.input, state.input_cursor, max_width);
    let input_line = build_input_line(&display, cursor_col, state.input_blink_on);
    let input = Paragraph::new(input_line).block(input_block);
    frame.render_widget(input, chunks[idx]);
    idx += 1;
    frame.render_widget(Paragraph::new(status_line(state)), chunks[idx]);
}

fn input_title(state: &ChatUiState) -> Line<'static> {
    if state.busy {
        Line::from(vec![
            Span::raw("Message "),
            Span::styled(
                format!("({})", activity_status(state)),
                Style::default().fg(Color::Yellow),
            ),
        ])
    } else {
        Line::from("Message")
    }
}

fn line_display_width(text: &str) -> usize {
    Line::from(text).width()
}

fn input_viewport(prompt: &str, input: &str, cursor: usize, max_width: usize) -> (String, usize) {
    let chars: Vec<char> = input.chars().collect();
    let cursor = cursor.min(chars.len());

    for skip in 0..=cursor.min(chars.len()) {
        let visible: String = chars[skip..].iter().collect();
        let display = format!("{prompt}{visible}");
        if line_display_width(&display) > max_width {
            continue;
        }
        let before: String = chars[skip..cursor].iter().collect();
        let col = line_display_width(&format!("{prompt}{before}"));
        if col <= max_width {
            return (display, col);
        }
    }

    for skip in 0..chars.len() {
        let visible: String = chars[skip..].iter().collect();
        let display = format!("{prompt}{visible}");
        if line_display_width(&display) <= max_width {
            let before: String = chars[skip..cursor.min(chars.len())].iter().collect();
            let col = line_display_width(&format!("{prompt}{before}"));
            return (display, col.min(max_width));
        }
    }

    (prompt.to_string(), prompt.len().min(max_width))
}

fn build_input_line(display: &str, cursor_col: usize, blink_on: bool) -> Line<'static> {
    let (before, after) = split_line_at_display_col(display, cursor_col);
    let cursor_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::REVERSED);
    if blink_on {
        let cursor_cell = after
            .chars()
            .next()
            .map(|ch| ch.to_string())
            .unwrap_or_else(|| " ".to_string());
        let rest: String = after.chars().skip(1).collect();
        Line::from(vec![
            Span::raw(before),
            Span::styled(cursor_cell, cursor_style),
            Span::raw(rest),
        ])
    } else {
        Line::from(vec![
            Span::raw(before),
            Span::styled("▏", Style::default().fg(Color::Cyan)),
            Span::raw(after),
        ])
    }
}

fn split_line_at_display_col(text: &str, target_col: usize) -> (String, String) {
    if target_col == 0 {
        return (String::new(), text.to_string());
    }
    let mut col = 0usize;
    let mut split_at = 0usize;
    for (idx, ch) in text.char_indices() {
        let ch_width = line_display_width(&ch.to_string());
        if col + ch_width > target_col {
            break;
        }
        col += ch_width;
        split_at = idx + ch.len_utf8();
        if col >= target_col {
            break;
        }
    }
    let before = text[..split_at].to_string();
    let after = text[split_at..].to_string();
    (before, after)
}

fn render_header(frame: &mut ratatui::Frame, area: Rect, state: &ChatUiState) {
    let thinking_fold = if state.thinking_expanded { "expanded" } else { "folded" };
    let tool_fold = if state.tools_expanded { "expanded" } else { "folded" };
    let context_used = state.last_prompt_tokens.max(state.estimated_session_tokens);
    let context_pct = usage_percent(context_used, state.context_tokens);
    let context_style = if context_pct >= 90 {
        Style::default().fg(Color::Red)
    } else if context_pct >= 75 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Green)
    };
    let cache_label = cache_hit_label(state);
    let activity = activity_status(state);
    let activity_style = if state.busy {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let text = vec![
        Line::from(vec![
            Span::styled(">_ Agnes Chat", Style::default().fg(Color::Cyan)),
            Span::raw("  PI agent for Agnes media + coding"),
        ]),
        Line::from(vec![
            Span::styled("model: ", Style::default().fg(Color::DarkGray)),
            Span::raw(state.model_label.as_str()),
            Span::raw("    "),
            Span::styled("thinking: ", Style::default().fg(Color::DarkGray)),
            Span::raw(if state.thinking_enabled { "on" } else { "off" }),
            Span::raw(format!(" ({thinking_fold})")),
            Span::raw("    "),
            Span::styled("approval: ", Style::default().fg(Color::DarkGray)),
            Span::raw(state.approval_label.as_str()),
        ]),
        Line::from(vec![
            Span::styled("context: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "{}/{} ({}%)",
                    format_u64_tokens(context_used),
                    format_tokens(state.context_tokens),
                    context_pct
                ),
                context_style,
            ),
            Span::raw("    "),
            Span::styled("output: ", Style::default().fg(Color::DarkGray)),
            Span::raw(format_tokens(state.max_output_tokens)),
        ]),
        Line::from(vec![
            Span::styled("cache hit: ", Style::default().fg(Color::DarkGray)),
            Span::raw(cache_label),
            Span::raw("    "),
            Span::styled("tools: ", Style::default().fg(Color::DarkGray)),
            Span::raw(tool_fold),
        ]),
        Line::from(vec![
            Span::styled("activity: ", Style::default().fg(Color::DarkGray)),
            Span::styled(activity, activity_style),
        ]),
        Line::from(vec![
            Span::styled("directory: ", Style::default().fg(Color::DarkGray)),
            Span::raw(state.directory_label.as_str()),
        ]),
        Line::from(vec![
            Span::styled("Tip: ", Style::default().fg(Color::Yellow)),
            Span::raw("Shift-Tab approval. ←/→ approve. Tab slash. Ctrl-R retry. PgUp/PgDn scroll. /compress context."),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_transcript(frame: &mut ratatui::Frame, area: Rect, state: &mut ChatUiState) {
    let inner_width = area.width.saturating_sub(2).max(20) as usize;
    let inner_height = area.height.saturating_sub(2).max(1) as usize;
    state.transcript_viewport = inner_height as u16;

    let all_lines = if state.lines.is_empty() {
        build_intro_lines(state, inner_width)
    } else {
        build_transcript_lines(state, inner_width)
    };
    let visible = slice_transcript_view(all_lines, inner_height, state.scroll_from_bottom);

    frame.render_widget(
        Paragraph::new(visible)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(Span::styled(" Chat ", Style::default().fg(Color::Cyan))),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_approval(frame: &mut ratatui::Frame, area: Rect, state: &mut ChatUiState) {
    let (tool_name, args) = match &state.pending_approval {
        Some(request) => (request.tool_name.clone(), request.args.clone()),
        None => return,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(
            " Approval required ",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(inner);

    let content_width = chunks[0].width.saturating_sub(2).max(20) as usize;
    let content_height = chunks[0].height.saturating_sub(0).max(1) as usize;
    state.approval_viewport = content_height as u16;

    let args_pretty = serde_json::to_string_pretty(&args).unwrap_or_else(|_| args.to_string());
    let mut content_lines = vec![
        Line::from(vec![
            Span::styled("Tool  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                tool_name,
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled("Arguments", Style::default().fg(Color::DarkGray))),
    ];
    for segment in wrap_text(&args_pretty, content_width) {
        content_lines.push(Line::from(Span::styled(segment, Style::default().fg(Color::Gray))));
    }
    let visible_content = slice_transcript_view(content_lines, content_height, state.approval_scroll_from_bottom);
    frame.render_widget(Paragraph::new(visible_content).wrap(Wrap { trim: false }), chunks[0]);

    let action_line = render_approval_actions(state.approval_choice);
    let hint = Line::from(Span::styled(
        "↑/↓ scroll args  ←/→ select  Enter confirm  y/a/n shortcuts",
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(Paragraph::new(vec![action_line, hint]), chunks[1]);
}

fn render_approval_actions(selected: usize) -> Line<'static> {
    let options = [("Approve", Color::Green), ("Allow session", Color::Cyan), ("Deny", Color::Red)];
    let mut spans = vec![Span::raw(" ")];
    for (index, (label, color)) in options.iter().enumerate() {
        let style = if index == selected {
            Style::default()
                .fg(Color::Black)
                .bg(*color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(*color)
        };
        spans.push(Span::styled(format!(" {label} "), style));
    }
    Line::from(spans)
}

fn render_completion(frame: &mut ratatui::Frame, area: Rect, state: &ChatUiState) {
    let text = state.completion_hint.as_deref().unwrap_or("");
    frame.render_widget(
        Paragraph::new(text)
            .style(Style::default().fg(Color::DarkGray))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn slice_transcript_view(
    lines: Vec<Line<'static>>,
    viewport_h: usize,
    scroll_from_bottom: usize,
) -> Vec<Line<'static>> {
    let total = lines.len();
    let end = total.saturating_sub(scroll_from_bottom);
    let start = end.saturating_sub(viewport_h);
    lines[start..end].to_vec()
}

fn build_intro_lines(state: &ChatUiState, width: usize) -> Vec<Line<'static>> {
    let style = Style::default().fg(Color::DarkGray);
    wrap_text(&intro_text(state), width)
        .into_iter()
        .map(|line| Line::from(Span::styled(line, style)))
        .collect()
}

fn build_transcript_lines(state: &ChatUiState, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut thinking_hidden = 0usize;
    let mut tools_hidden = 0usize;

    for line in &state.lines {
        match line.kind {
            ChatLineKind::Thinking if !state.thinking_expanded => {
                thinking_hidden += 1;
            }
            ChatLineKind::Tool if !state.tools_expanded => {
                tools_hidden += 1;
            }
            ChatLineKind::User => {
                flush_hidden_thinking(&mut out, thinking_hidden);
                thinking_hidden = 0;
                flush_hidden_tools(&mut out, tools_hidden);
                tools_hidden = 0;
                push_turn_divider(&mut out, width);
                out.extend(render_user_bubble(&line.text, width));
            }
            ChatLineKind::Assistant => {
                flush_hidden_thinking(&mut out, thinking_hidden);
                thinking_hidden = 0;
                flush_hidden_tools(&mut out, tools_hidden);
                tools_hidden = 0;
                out.extend(render_assistant_markdown(&line.text, width));
            }
            ChatLineKind::Thinking => {
                flush_hidden_tools(&mut out, tools_hidden);
                tools_hidden = 0;
                thinking_hidden = 0;
                out.extend(render_styled_prefixed(
                    "  thinking: ",
                    &line.text,
                    width,
                    Style::default().fg(Color::DarkGray),
                ));
            }
            ChatLineKind::Tool => {
                flush_hidden_thinking(&mut out, thinking_hidden);
                thinking_hidden = 0;
                tools_hidden = 0;
                out.extend(render_styled_prefixed(
                    "  tool: ",
                    &line.text,
                    width,
                    Style::default().fg(Color::Yellow),
                ));
            }
            ChatLineKind::Status => {
                out.extend(render_styled_prefixed(
                    "  ",
                    &line.text,
                    width,
                    Style::default().fg(Color::Cyan),
                ));
            }
            ChatLineKind::Error => {
                out.extend(render_styled_prefixed(
                    "! ",
                    &line.text,
                    width,
                    Style::default().fg(Color::Red),
                ));
            }
        }
    }
    flush_hidden_thinking(&mut out, thinking_hidden);
    flush_hidden_tools(&mut out, tools_hidden);
    while out
        .last()
        .is_some_and(|line| line.spans.is_empty() || line_is_blank(line))
    {
        out.pop();
    }
    out
}

fn flush_hidden_thinking(out: &mut Vec<Line<'static>>, count: usize) {
    if count == 0 {
        return;
    }
    let text = if count == 1 {
        "• Thinking hidden (Ctrl-T to expand)".to_string()
    } else {
        format!("• Thinking hidden ×{count} (Ctrl-T to expand)")
    };
    out.push(Line::from(Span::styled(text, Style::default().fg(Color::DarkGray))));
}

fn flush_hidden_tools(out: &mut Vec<Line<'static>>, count: usize) {
    if count == 0 {
        return;
    }
    let text = if count == 1 {
        "• Tool activity hidden (Ctrl-E to expand)".to_string()
    } else {
        format!("• Tool activity hidden ×{count} (Ctrl-E to expand)")
    };
    out.push(Line::from(Span::styled(text, Style::default().fg(Color::DarkGray))));
}

fn push_turn_divider(out: &mut Vec<Line<'static>>, width: usize) {
    if out.is_empty() {
        return;
    }
    out.push(dashed_divider(width));
}

fn dashed_divider(width: usize) -> Line<'static> {
    let width = width.max(8);
    let unit = "─ ";
    let mut line = String::new();
    while line.chars().count() < width {
        line.push_str(unit);
    }
    let trimmed: String = line.chars().take(width).collect();
    Line::from(Span::styled(trimmed, Style::default().fg(Color::DarkGray)))
}

fn line_is_blank(line: &Line) -> bool {
    line.spans.iter().all(|span| span.content.trim().is_empty())
}

fn render_user_bubble(text: &str, width: usize) -> Vec<Line<'static>> {
    let bubble_width = (width * USER_MAX_WIDTH_PERCENT / 100).max(20);
    let style = Style::default().bg(USER_BUBBLE_BG).fg(Color::White);
    wrap_text(text, bubble_width)
        .into_iter()
        .map(|line| Line::from(Span::styled(format!(" {line} "), style)))
        .collect()
}

fn convert_core_color(color: ratatui_core::style::Color) -> Color {
    use ratatui_core::style::Color as C;
    match color {
        C::Reset => Color::Reset,
        C::Black => Color::Black,
        C::Red => Color::Red,
        C::Green => Color::Green,
        C::Yellow => Color::Yellow,
        C::Blue => Color::Blue,
        C::Magenta => Color::Magenta,
        C::Cyan => Color::Cyan,
        C::Gray => Color::Gray,
        C::DarkGray => Color::DarkGray,
        C::LightRed => Color::LightRed,
        C::LightGreen => Color::LightGreen,
        C::LightYellow => Color::LightYellow,
        C::LightBlue => Color::LightBlue,
        C::LightMagenta => Color::LightMagenta,
        C::LightCyan => Color::LightCyan,
        C::White => Color::White,
        C::Rgb(r, g, b) => Color::Rgb(r, g, b),
        C::Indexed(i) => Color::Indexed(i),
    }
}

fn convert_core_style(style: ratatui_core::style::Style) -> Style {
    Style {
        fg: style.fg.map(convert_core_color),
        bg: style.bg.map(convert_core_color),
        add_modifier: Modifier::from_bits_truncate(style.add_modifier.bits()),
        sub_modifier: Modifier::from_bits_truncate(style.sub_modifier.bits()),
        ..Style::default()
    }
}

fn render_assistant_markdown(text: &str, width: usize) -> Vec<Line<'static>> {
    let md = tui_markdown::from_str(text);
    let mut out = Vec::new();
    for (idx, line) in md.lines.into_iter().enumerate() {
        let converted = Line::from(
            line.spans
                .into_iter()
                .map(|span| Span::styled(span.content.into_owned(), convert_core_style(span.style)))
                .collect::<Vec<_>>(),
        );
        let mut wrapped = wrap_styled_line(converted, width.saturating_sub(2));
        if idx == 0 && !wrapped.is_empty() {
            let first = wrapped.remove(0);
            let mut spans = vec![Span::styled("● ", Style::default().fg(Color::Green))];
            spans.extend(first.spans);
            wrapped.insert(0, Line::from(spans));
        }
        out.extend(wrapped);
    }
    if out.is_empty() {
        out.push(Line::from(Span::styled("● ", Style::default().fg(Color::Green))));
    }
    out
}

fn render_styled_prefixed(prefix: &str, text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    wrap_prefixed(prefix, text, width)
        .into_iter()
        .map(|line| Line::from(Span::styled(line, style)))
        .collect()
}

fn wrap_styled_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if line.width() <= width {
        return vec![line];
    }
    let plain: String = line.spans.iter().map(|span| span.content.as_ref()).collect();
    wrap_text(&plain, width).into_iter().map(Line::from).collect()
}

fn intro_text(state: &ChatUiState) -> String {
    format!(
        "Welcome to Agnes Chat.\n\nAsk for code changes, image generation, async video tasks, asset/history lookup, or skill loading.\n\nTab completes slash commands. /retry or Ctrl-R retries the previous request without adding another user message to the session. Ctrl-T toggles thinking visibility. Ctrl-E toggles tool details. Current approval mode: {}.",
        state.approval_label
    )
}

fn wrap_prefixed(prefix: &str, text: &str, width: usize) -> Vec<String> {
    let available = width.saturating_sub(prefix.chars().count()).max(8);
    let wrapped = wrap_text(text, available);
    let indent = " ".repeat(prefix.chars().count());
    wrapped
        .into_iter()
        .enumerate()
        .map(|(idx, line)| {
            if idx == 0 {
                format!("{prefix}{line}")
            } else {
                format!("{indent}{line}")
            }
        })
        .collect()
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for raw_line in text.lines() {
        if raw_line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in raw_line.split_whitespace() {
            let word_len = word.chars().count();
            let line_len = line.chars().count();
            if line.is_empty() {
                if word_len <= width {
                    line.push_str(word);
                } else {
                    out.extend(split_long_word(word, width));
                }
            } else if line_len + 1 + word_len <= width {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(line);
                line = String::new();
                if word_len <= width {
                    line.push_str(word);
                } else {
                    out.extend(split_long_word(word, width));
                }
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    out
}

fn split_long_word(word: &str, width: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in word.chars() {
        if current.chars().count() >= width {
            chunks.push(current);
            current = String::new();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn status_line(state: &ChatUiState) -> String {
    format!(
        "{}  |  approval: {}  |  {}  |  Ctrl-Q quit",
        state.status,
        state.approval_label,
        activity_status(state)
    )
}

fn activity_status(state: &ChatUiState) -> String {
    if state.pending_approval.is_some() {
        return format!(
            "approval pending: {}",
            state
                .pending_approval
                .as_ref()
                .map(|request| request.tool_name.as_str())
                .unwrap_or("tool")
        );
    }
    if !state.busy {
        return "idle".into();
    }
    match &state.run_phase {
        RunPhase::Idle => "running".into(),
        RunPhase::Waiting => "waiting for model".into(),
        RunPhase::Thinking => "thinking".into(),
        RunPhase::Responding => "responding".into(),
        RunPhase::Tool(name) => format!("tool: {name}"),
    }
}

fn format_tokens(tokens: u32) -> String {
    if tokens.is_multiple_of(1024) {
        format!("{}k", tokens / 1024)
    } else {
        tokens.to_string()
    }
}

fn format_u64_tokens(tokens: u64) -> String {
    format_tokens(tokens.min(u64::from(u32::MAX)) as u32)
}

fn approval_label_for(auto_mode: bool) -> String {
    if auto_mode {
        "auto, dangerous calls still reviewed".into()
    } else {
        "review".into()
    }
}

fn cache_hit_label(state: &ChatUiState) -> String {
    if state.last_prompt_tokens == 0 {
        return "— (after first response)".into();
    }
    let pct = state.last_cache_read.saturating_mul(100) / state.last_prompt_tokens.max(1);
    format!(
        "{} / {} ({}%)",
        format_u64_tokens(state.last_cache_read),
        format_u64_tokens(state.last_prompt_tokens),
        pct
    )
}

struct UiPermission {
    workspace: PathBuf,
    mode: Mutex<ApprovalMode>,
    tx: mpsc::UnboundedSender<UiEvent>,
    allowed_session: Mutex<std::collections::HashSet<String>>,
}

impl UiPermission {
    fn new(workspace: PathBuf, mode: ApprovalMode, tx: mpsc::UnboundedSender<UiEvent>) -> Self {
        Self { workspace, mode: Mutex::new(mode), tx, allowed_session: Mutex::new(Default::default()) }
    }

    fn set_mode(&self, mode: ApprovalMode) {
        if let Ok(mut current) = self.mode.lock() {
            *current = mode;
        }
    }

    fn current_policy(&self) -> ApprovalPolicy {
        let mode = self.mode.lock().map(|guard| *guard).unwrap_or(ApprovalMode::Review);
        match mode {
            ApprovalMode::Auto => ApprovalPolicy::auto(self.workspace.clone()),
            ApprovalMode::Review => ApprovalPolicy::default_review(self.workspace.clone()),
        }
    }
}

#[async_trait]
impl PermissionPolicy for UiPermission {
    async fn check(&self, tool_name: &str, args: &Value) -> PermissionDecision {
        if let Ok(allowed) = self.allowed_session.lock()
            && allowed.contains(tool_name)
        {
            return PermissionDecision::Allow;
        }
        if self.current_policy().classify(tool_name, args) == ApprovalDecision::Allow {
            let _ = self.tx.send(UiEvent::AutoApproved { tool_name: tool_name.to_string() });
            return PermissionDecision::Allow;
        }
        let (respond_to, response) = oneshot::channel();
        let request = ApprovalRequest { tool_name: tool_name.to_string(), args: args.clone(), respond_to };
        if self.tx.send(UiEvent::Approval(request)).is_err() {
            return PermissionDecision::Deny { reason: "approval UI unavailable".into() };
        }
        match response.await {
            Ok(PermissionDecision::AllowSession) => {
                if let Ok(mut allowed) = self.allowed_session.lock() {
                    allowed.insert(tool_name.to_string());
                }
                PermissionDecision::AllowSession
            }
            Ok(decision) => decision,
            Err(_) => PermissionDecision::Deny { reason: "approval cancelled".into() },
        }
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        format!(
            "{}...",
            text.chars().take(max_chars.saturating_sub(3)).collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transcript_line_texts(state: &ChatUiState, width: usize) -> Vec<String> {
        build_transcript_lines(state, width)
            .into_iter()
            .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>())
            .collect()
    }

    #[test]
    fn streaming_text_appends_to_assistant_line() {
        let mut state = ChatUiState::default();

        state.push_assistant_delta("hello");
        state.push_assistant_delta(" world");

        assert_eq!(state.lines.len(), 1);
        assert_eq!(state.lines[0].kind, ChatLineKind::Assistant);
        assert_eq!(state.lines[0].text, "hello world");
    }

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
        let options = ChatUiOptions { resume: None, auto: false, max_turns: 4, overrides: ChatOverrides::default() };

        handle_slash_command(&mut state, &AppConfig::default(), &options, "/skills").unwrap();

        assert!(
            state
                .lines
                .iter()
                .any(|line| line.text.contains("agnes-aigc-gen") && line.text.contains("Generates images and videos"))
        );
    }

    #[test]
    fn skill_description_parses_folded_front_matter() {
        let text = "---\nname: demo\ndescription: >-\n  Does one thing.\n  Also does another.\n---\n# Demo\n";

        assert_eq!(
            skill_description_from_markdown(text).as_deref(),
            Some("Does one thing. Also does another.")
        );
    }

    #[test]
    fn wraps_long_text_to_available_width() {
        let wrapped = wrap_text("one two three four five", 9);

        assert_eq!(wrapped, vec!["one two", "three", "four five"]);
    }

    #[test]
    fn thinking_is_collapsed_by_default_and_can_expand() {
        let mut state = ChatUiState::default();
        state.push_thinking_delta("private reasoning details");

        assert!(
            transcript_line_texts(&state, 80)
                .iter()
                .any(|line| line.contains("Thinking hidden"))
        );
        assert!(
            !transcript_line_texts(&state, 80)
                .iter()
                .any(|line| line.contains("private reasoning details"))
        );

        state.toggle_thinking();

        assert!(
            transcript_line_texts(&state, 80)
                .iter()
                .any(|line| line.contains("private reasoning details"))
        );
    }

    #[test]
    fn tools_are_collapsed_by_default_and_can_expand() {
        let mut state = ChatUiState::default();
        state.push_line(ChatLineKind::Tool, "agnes_submit_video({\"prompt\":\"test\"})");

        assert!(
            transcript_line_texts(&state, 80)
                .iter()
                .any(|line| line.contains("Tool activity hidden"))
        );
        assert!(
            !transcript_line_texts(&state, 80)
                .iter()
                .any(|line| line.contains("agnes_submit_video"))
        );

        state.toggle_tools();

        assert!(
            transcript_line_texts(&state, 80)
                .iter()
                .any(|line| line.contains("agnes_submit_video"))
        );
    }

    #[test]
    fn tab_completes_slash_command() {
        let mut state = ChatUiState { input: "/thi".into(), ..ChatUiState::default() };

        state.complete_command();

        assert_eq!(state.input, "/thinking ");
    }

    #[test]
    fn tab_completes_skill_name() {
        let mut state = ChatUiState { input: "/skill agnes".into(), ..ChatUiState::default() };

        state.complete_command();

        assert_eq!(state.input, "/skill agnes-aigc-gen ");
    }

    #[test]
    fn manual_retry_reuses_saved_history_without_duplicate_user_prompt() {
        let mut state = ChatUiState::default();
        let mut session = ChatSession::new("agnes-2.0-flash", true);
        session.messages = vec![
            Message::user_text("previous context"),
            Message::user_text("retry me"),
            Message::user_text("retry me"),
        ];
        state.session = Some(session);
        state.last_request = Some(RetryRequest {
            input: "retry me".into(),
            history: vec![Message::user_text("previous context"), Message::user_text("retry me")],
        });

        let retry = state.last_request_for_retry().unwrap();

        assert_eq!(retry.history.len(), 2);
        assert_eq!(user_message_count(&retry.history, "retry me"), 1);
    }

    #[test]
    fn slice_transcript_respects_scroll_offset() {
        let lines: Vec<Line> = (0..10).map(|idx| Line::from(format!("line {idx}"))).collect();

        let visible = slice_transcript_view(lines, 3, 0);
        assert_eq!(visible.len(), 3);
        assert!(visible[2].to_string().contains("line 9"));

        let visible = slice_transcript_view((0..10).map(|idx| Line::from(format!("line {idx}"))).collect(), 3, 2);
        assert!(visible[2].to_string().contains("line 7"));
    }

    #[test]
    fn user_bubble_is_left_aligned_with_background() {
        let lines = render_user_bubble("hello", 40);
        let bubble = lines
            .iter()
            .find(|line| line.spans.iter().any(|span| span.content.contains("hello")))
            .expect("bubble line");
        assert_eq!(bubble.spans.len(), 1);
        assert_eq!(bubble.spans[0].style.bg, Some(USER_BUBBLE_BG));
        assert!(bubble.spans[0].content.contains("hello"));
    }

    #[test]
    fn input_history_recalls_previous_entries() {
        let mut state = ChatUiState::default();
        state.record_input_history("first prompt");
        state.record_input_history("second prompt");

        state.input_history_prev();
        assert_eq!(state.input, "second prompt");

        state.input_history_prev();
        assert_eq!(state.input, "first prompt");

        state.input_history_next();
        assert_eq!(state.input, "second prompt");

        state.input_history_next();
        assert_eq!(state.input, "");
    }

    #[test]
    fn input_history_restores_draft_after_browsing() {
        let mut state = ChatUiState { input: "draft".into(), ..ChatUiState::default() };
        state.record_input_history("earlier");

        state.input_history_prev();
        assert_eq!(state.input, "earlier");

        state.input_history_next();
        assert_eq!(state.input, "draft");
    }

    #[test]
    fn input_history_skips_duplicate_submissions() {
        let mut state = ChatUiState::default();
        state.record_input_history("same");
        state.record_input_history("same");

        assert_eq!(state.input_history.len(), 1);
    }

    #[test]
    fn pinned_to_bottom_resets_scroll_on_new_content() {
        let mut state = ChatUiState { pinned_to_bottom: true, scroll_from_bottom: 5, ..Default::default() };
        state.push_line(ChatLineKind::Status, "new");

        assert_eq!(state.scroll_offset(), 0);
    }

    #[test]
    fn unpinned_scroll_keeps_offset_on_new_content() {
        let mut state = ChatUiState { pinned_to_bottom: false, scroll_from_bottom: 5, ..Default::default() };
        state.push_line(ChatLineKind::Status, "new");

        assert_eq!(state.scroll_offset(), 5);
    }

    #[test]
    fn assistant_markdown_renders_bold() {
        let lines = render_assistant_markdown("**bold**", 80);
        assert!(lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|span| span.content.contains("bold") && span.style.add_modifier.contains(Modifier::BOLD))
        }));
    }

    #[test]
    fn approval_choice_cycles_with_left_and_right() {
        let mut state = ChatUiState::default();
        assert_eq!(state.approval_choice_index(), 0);
        state.approval_choice_right();
        assert_eq!(state.approval_choice_index(), 1);
        state.approval_choice_left();
        assert_eq!(state.approval_choice_index(), 0);
        state.approval_choice_left();
        assert_eq!(state.approval_choice_index(), 2);
    }

    #[test]
    fn transcript_uses_dashed_dividers_between_turns() {
        let mut state = ChatUiState::default();
        state.push_user("hello");
        state.push_line(ChatLineKind::Thinking, "hmm");
        state.push_line(ChatLineKind::Assistant, "world");
        state.push_user("again");

        let lines = transcript_line_texts(&state, 40);
        let divider_count = lines.iter().filter(|line| line.contains('─')).count();
        assert_eq!(divider_count, 1, "only divider between turns: {lines:?}");
    }

    #[test]
    fn thinking_and_tools_skip_dividers_within_a_turn() {
        let mut state = ChatUiState { tools_expanded: true, thinking_expanded: true, ..ChatUiState::default() };
        state.push_user("hello");
        state.push_line(ChatLineKind::Thinking, "plan");
        state.push_line(ChatLineKind::Tool, "→ run");
        state.push_line(ChatLineKind::Assistant, "done");

        let lines = transcript_line_texts(&state, 40);
        assert_eq!(lines.iter().filter(|line| line.contains('─')).count(), 0);
    }

    #[test]
    fn input_cursor_accounts_for_wide_characters() {
        let (_, col) = input_viewport("> ", "帮我打开", 4, 80);
        assert_eq!(col, 10);
    }

    #[test]
    fn input_cursor_moves_with_left_and_right() {
        let mut state = ChatUiState { input: "abcd".into(), input_cursor: 4, ..ChatUiState::default() };
        state.input_cursor_left();
        assert_eq!(state.input_cursor, 3);
        state.input_insert('X');
        assert_eq!(state.input, "abcXd");
        assert_eq!(state.input_cursor, 4);
    }

    #[test]
    fn input_line_scrolls_to_show_tail_when_overflowing() {
        let long = "a".repeat(200);
        let display = input_viewport("> ", &long, long.chars().count(), 20).0;
        assert!(line_display_width(&display) <= 20);
        assert!(display.ends_with('a'));
    }

    #[test]
    fn intro_text_explains_the_interface() {
        let state = ChatUiState::default();

        assert!(intro_text(&state).contains("Tab completes slash commands"));
        assert!(intro_text(&state).contains("Ctrl-T"));
    }

    fn user_message_count(messages: &[Message], needle: &str) -> usize {
        messages
            .iter()
            .filter(|message| match message {
                Message::User { content, .. } => content.iter().any(|content| content.as_text() == Some(needle)),
                _ => false,
            })
            .count()
    }
}
