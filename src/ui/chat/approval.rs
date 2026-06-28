use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use pi_agent::{PermissionDecision, PermissionPolicy};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::agent::approval::{ApprovalDecision, ApprovalMode, ApprovalPolicy};

use super::model::{ApprovalRequest, UiEvent};

pub(crate) const APPROVAL_CHOICE_COUNT: usize = 3;

#[derive(Default)]
pub(crate) struct ApprovalPanel {
    pub pending: Option<ApprovalRequest>,
    pub choice: usize,
    pub scroll_from_bottom: usize,
    pub viewport: u16,
}

impl ApprovalPanel {
    pub fn is_active(&self) -> bool {
        self.pending.is_some()
    }

    pub fn begin(&mut self, request: ApprovalRequest) {
        self.choice = 0;
        self.scroll_from_bottom = 0;
        self.pending = Some(request);
    }

    pub fn choice_left(&mut self) {
        self.choice = (self.choice + APPROVAL_CHOICE_COUNT - 1) % APPROVAL_CHOICE_COUNT;
    }

    pub fn choice_right(&mut self) {
        self.choice = (self.choice + 1) % APPROVAL_CHOICE_COUNT;
    }

    #[cfg(test)]
    pub fn choice_index(&self) -> usize {
        self.choice
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_add(lines);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(lines);
    }

    pub fn confirm_choice(&self) -> PermissionDecision {
        match self.choice {
            0 => PermissionDecision::Allow,
            1 => PermissionDecision::AllowSession,
            _ => PermissionDecision::Deny { reason: "user denied".into() },
        }
    }
}

pub(crate) struct UiPermission {
    workspace: PathBuf,
    mode: Mutex<ApprovalMode>,
    tx: mpsc::UnboundedSender<UiEvent>,
    allowed_session: Mutex<std::collections::HashSet<String>>,
}

impl UiPermission {
    pub(crate) fn new(workspace: PathBuf, mode: ApprovalMode, tx: mpsc::UnboundedSender<UiEvent>) -> Self {
        Self { workspace, mode: Mutex::new(mode), tx, allowed_session: Mutex::new(Default::default()) }
    }

    pub(crate) fn set_mode(&self, mode: ApprovalMode) {
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

pub(crate) fn render_approval_actions(selected: usize) -> Line<'static> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_choice_cycles_with_left_and_right() {
        let mut panel = ApprovalPanel::default();
        assert_eq!(panel.choice_index(), 0);
        panel.choice_right();
        assert_eq!(panel.choice_index(), 1);
        panel.choice_left();
        assert_eq!(panel.choice_index(), 0);
        panel.choice_left();
        assert_eq!(panel.choice_index(), 2);
    }
}
