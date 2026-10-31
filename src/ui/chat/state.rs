use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use pi_agent::PermissionDecision;
use pi_ai::Message;

use crate::agent::approval::ApprovalMode;
use crate::agent::chat::ChatOverrides;

use super::approval::ApprovalPanel;
use super::commands::push_help_lines;
use super::input::InputState;
use super::model::{ApprovalRequest, ChatLine, ChatLineKind, RetryRequest, RunPhase};
use super::scroll::ScrollView;
use super::status::approval_label_for;
use super::telemetry::SessionTelemetry;

pub struct ChatUiState {
    pub lines: Vec<ChatLine>,
    pub(crate) input: InputState,
    pub(crate) transcript: ScrollView,
    pub(crate) approval: ApprovalPanel,
    pub(crate) telemetry: SessionTelemetry,
    pub(crate) status: String,
    pub(crate) session: Option<crate::agent::session::ChatSession>,
    pub(crate) busy: bool,
    pub(crate) last_request: Option<RetryRequest>,
    pub(crate) loaded_skills: Vec<String>,
    pub(crate) overrides: ChatOverrides,
    pub(crate) thinking_expanded: bool,
    pub(crate) tools_expanded: bool,
    pub(crate) auto_mode: bool,
    pub(crate) permission: Option<Arc<super::approval::UiPermission>>,
    pub(crate) run_phase: RunPhase,
}

impl Default for ChatUiState {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            input: InputState::default(),
            transcript: ScrollView::default(),
            approval: ApprovalPanel::default(),
            telemetry: SessionTelemetry::default(),
            status: String::new(),
            session: None,
            busy: false,
            last_request: None,
            loaded_skills: Vec::new(),
            overrides: ChatOverrides::default(),
            thinking_expanded: false,
            tools_expanded: false,
            auto_mode: false,
            permission: None,
            run_phase: RunPhase::Idle,
        }
    }
}

impl ChatUiState {
    pub fn push_line(&mut self, kind: ChatLineKind, text: impl Into<String>) {
        self.lines.push(ChatLine { kind, text: text.into() });
        self.transcript.on_new_content();
    }

    pub fn push_user(&mut self, text: impl Into<String>) {
        self.push_line(ChatLineKind::User, text);
    }

    pub fn push_assistant_delta(&mut self, delta: &str) {
        if let Some(last) = self.lines.last_mut()
            && last.kind == ChatLineKind::Assistant
        {
            last.text.push_str(delta);
            self.transcript.on_new_content();
            return;
        }
        self.push_line(ChatLineKind::Assistant, delta);
    }

    pub fn push_thinking_delta(&mut self, delta: &str) {
        if let Some(last) = self.lines.last_mut()
            && last.kind == ChatLineKind::Thinking
        {
            last.text.push_str(delta);
            self.transcript.on_new_content();
            return;
        }
        self.push_line(ChatLineKind::Thinking, delta);
    }

    #[cfg(test)]
    pub(crate) fn scroll_offset(&self) -> usize {
        self.transcript.from_bottom
    }

    pub(crate) fn handle_scroll_key(&mut self, key: KeyCode, mods: KeyModifiers) {
        self.transcript.handle_key(key, mods);
    }

    pub(crate) fn record_input_history(&mut self, input: &str) {
        self.input.record_history(input);
    }

    pub(crate) fn input_history_prev(&mut self) {
        self.input.history_prev();
    }

    pub(crate) fn input_history_next(&mut self) {
        self.input.history_next();
    }

    pub(crate) fn reset_input_history_navigation(&mut self) {
        self.input.reset_history_navigation();
    }

    pub(crate) fn set_input_cursor(&mut self, cursor: usize) {
        self.input.set_cursor(cursor);
    }

    pub(crate) fn input_cursor_left(&mut self) {
        self.input.cursor_left();
    }

    pub(crate) fn input_cursor_right(&mut self) {
        self.input.cursor_right();
    }

    pub(crate) fn input_insert(&mut self, ch: char) {
        self.input.insert(ch);
    }

    pub(crate) fn input_backspace(&mut self) {
        self.input.backspace();
    }

    pub(crate) fn input_delete(&mut self) {
        self.input.delete();
    }

    pub(crate) fn sync_input_cursor_end(&mut self) {
        self.input.sync_cursor_end();
    }

    pub(crate) fn tick_input_blink(&mut self) {
        self.input.tick_blink();
    }

    pub(crate) fn toggle_approval_mode(&mut self) {
        self.auto_mode = !self.auto_mode;
        self.telemetry.approval_label = approval_label_for(self.auto_mode);
        if let Some(permission) = &self.permission {
            permission.set_mode(if self.auto_mode {
                ApprovalMode::Auto
            } else {
                ApprovalMode::Review
            });
        }
        self.status = format!("Approval mode: {}", self.telemetry.approval_label);
    }

    pub(crate) fn begin_approval(&mut self, request: ApprovalRequest) {
        self.status = format!("Approval required: {}", request.tool_name);
        self.approval.begin(request);
    }

    pub(crate) fn approval_choice_left(&mut self) {
        self.approval.choice_left();
    }

    pub(crate) fn approval_choice_right(&mut self) {
        self.approval.choice_right();
    }

    pub(crate) fn approval_scroll_up(&mut self, lines: usize) {
        self.approval.scroll_up(lines);
    }

    pub(crate) fn approval_scroll_down(&mut self, lines: usize) {
        self.approval.scroll_down(lines);
    }

    pub(crate) fn confirm_approval_choice(&mut self) -> Option<PermissionDecision> {
        Some(self.approval.confirm_choice())
    }

    pub(crate) fn update_usage_from_assistant(&mut self, message: &Message) {
        if let Message::Assistant(assistant) = message {
            self.telemetry.last_prompt_tokens = assistant.usage.input;
            self.telemetry.last_cache_read = assistant.usage.cache_read;
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

    pub(crate) fn last_request_for_retry(&self) -> Option<RetryRequest> {
        self.last_request.clone()
    }

    pub fn complete_command(&mut self) {
        self.input.complete_command();
    }
}

#[cfg(test)]
mod tests {
    use pi_ai::Message;

    use super::*;
    use crate::ui::chat::model::RetryRequest;

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
    fn manual_retry_reuses_saved_history_without_duplicate_user_prompt() {
        let mut state = ChatUiState::default();
        let mut session = crate::agent::session::ChatSession::new("agnes-3.0-flash", true);
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
    fn pinned_to_bottom_resets_scroll_on_new_content() {
        let mut state = ChatUiState {
            transcript: ScrollView { pinned_to_bottom: true, from_bottom: 5, ..Default::default() },
            ..Default::default()
        };
        state.push_line(ChatLineKind::Status, "new");

        assert_eq!(state.scroll_offset(), 0);
    }

    #[test]
    fn unpinned_scroll_keeps_offset_on_new_content() {
        let mut state = ChatUiState {
            transcript: ScrollView { pinned_to_bottom: false, from_bottom: 5, ..Default::default() },
            ..Default::default()
        };
        state.push_line(ChatLineKind::Status, "new");

        assert_eq!(state.scroll_offset(), 5);
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
