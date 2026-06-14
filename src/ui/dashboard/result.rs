use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::cli::list_tui::{copy_to_clipboard_silent, detail_field_tabs_line, detail_value_display};
use crate::db::VideoTaskRecord;
use crate::output::GenerationResult;

use super::task_display::{generation_result_from_task, progress_bar, progress_percent, status_icon, task_status_kind};

const RESULT_FIELDS: [&str; 6] = ["URI", "ASSET URI", "SIZE", "RATIO", "ID", "TYPE"];
const OUTPUT_IDLE_HINT: &str = "F5 submit  Tab cycles sections";
const OUTPUT_DONE_HINT: &str = "←→ field  Enter copy  c asset_uri  Tab next section";
const OUTPUT_FOCUS_HINT: &str = "←→ field  Enter copy  Tab next section";

pub struct ResultPanel {
    pub results: Vec<GenerationResult>,
    pub index: usize,
    pub field: usize,
    local_job: Option<String>,
    pending_id: Option<i64>,
    pending_status: String,
    pending_progress: Option<i32>,
    error_message: Option<String>,
    pulse: u64,
}

pub enum PendingSyncOutcome {
    Unchanged,
    Updated,
    Completed(GenerationResult),
    Failed(String),
}

impl ResultPanel {
    pub fn from_results(results: Vec<GenerationResult>) -> Self {
        Self {
            results,
            index: 0,
            field: 0,
            local_job: None,
            pending_id: None,
            pending_status: String::new(),
            pending_progress: None,
            error_message: None,
            pulse: 0,
        }
    }

    fn bump_pulse(&mut self) {
        self.pulse = self.pulse.wrapping_add(1);
    }

    pub fn clear_error(&mut self) {
        self.error_message = None;
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.local_job = None;
        self.pending_id = None;
        self.results.clear();
        self.error_message = Some(msg.into());
        self.bump_pulse();
    }

    pub fn set_ack(&mut self, msg: impl Into<String>) {
        self.error_message = None;
        self.local_job = Some(msg.into());
        self.pending_id = None;
        self.results.clear();
        self.bump_pulse();
    }

    pub fn set_local_job(&mut self, label: impl Into<String>) {
        self.error_message = None;
        self.local_job = Some(label.into());
        self.pending_id = None;
        self.pending_status.clear();
        self.pending_progress = None;
        self.results.clear();
        self.index = 0;
        self.field = 0;
        self.bump_pulse();
    }

    pub fn clear_local_job(&mut self) {
        self.local_job = None;
    }

    pub fn set_pending_task(&mut self, local_id: i64) {
        self.error_message = None;
        self.local_job = None;
        self.results.clear();
        self.index = 0;
        self.field = 0;
        self.pending_id = Some(local_id);
        self.pending_status = "queued".into();
        self.pending_progress = None;
        self.bump_pulse();
    }

    pub fn pending_task_id(&self) -> Option<i64> {
        self.pending_id
    }

    pub fn is_working(&self) -> bool {
        self.local_job.is_some() || self.pending_id.is_some()
    }

    pub fn can_navigate(&self) -> bool {
        !self.results.is_empty() && !self.is_working() && self.error_message.is_none()
    }

    pub fn sync_pending(&mut self, row: &VideoTaskRecord) -> PendingSyncOutcome {
        let Some(id) = self.pending_id else {
            return PendingSyncOutcome::Unchanged;
        };
        if row.id != id {
            return PendingSyncOutcome::Unchanged;
        }
        if row.phase == "success" {
            self.pending_id = None;
            if let Some(result) = generation_result_from_task(row) {
                self.results = vec![result.clone()];
                return PendingSyncOutcome::Completed(result);
            }
            return PendingSyncOutcome::Failed("task completed without video uri".into());
        }
        if row.phase == "failed" {
            self.pending_id = None;
            let msg = row
                .error
                .as_ref()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "video task failed".into());
            return PendingSyncOutcome::Failed(msg);
        }
        let changed = self.pending_status != row.status || self.pending_progress != row.progress;
        self.pending_status = row.status.clone();
        self.pending_progress = row.progress;
        if changed {
            PendingSyncOutcome::Updated
        } else {
            PendingSyncOutcome::Unchanged
        }
    }

    pub fn latest_asset_uri(&self) -> Option<String> {
        self.results
            .iter()
            .rev()
            .find_map(|r| r.asset_uri.clone())
            .filter(|s| !s.is_empty())
    }

    pub fn field_next(&mut self) {
        if self.pending_id.is_some() || self.local_job.is_some() {
            return;
        }
        self.field = (self.field + 1) % RESULT_FIELDS.len();
    }

    pub fn field_previous(&mut self) {
        if self.pending_id.is_some() || self.local_job.is_some() {
            return;
        }
        self.field = if self.field == 0 {
            RESULT_FIELDS.len() - 1
        } else {
            self.field - 1
        };
    }

    pub fn result_next(&mut self) {
        if self.results.len() > 1 && self.index + 1 < self.results.len() {
            self.index += 1;
        }
    }

    pub fn result_previous(&mut self) {
        if self.results.len() > 1 && self.index > 0 {
            self.index -= 1;
        }
    }

    fn current(&self) -> Option<&GenerationResult> {
        self.results.get(self.index)
    }

    fn field_value(&self, field: usize) -> String {
        let Some(result) = self.current() else {
            return "-".into();
        };
        match field {
            0 => result.uri.clone(),
            1 => result.asset_uri.clone().unwrap_or_else(|| "-".into()),
            2 => result.size.clone(),
            3 => result.ratio.clone(),
            4 => result
                .generation_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".into()),
            5 => result.kind.clone(),
            _ => "-".into(),
        }
    }

    pub fn selected_value(&self) -> String {
        self.field_value(self.field)
    }

    pub fn copy_field(&self) {
        let value = self.selected_value();
        if value != "-" {
            copy_to_clipboard_silent(&value);
        }
    }

    pub fn copy_asset_uri(&self) {
        if let Some(result) = self.current()
            && let Some(ref uri) = result.asset_uri
        {
            copy_to_clipboard_silent(uri);
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, tick: u64, focused: bool) {
        if let Some(ref err) = self.error_message {
            self.render_error(frame, area, err, focused);
            return;
        }
        if let Some(ref label) = self.local_job {
            self.render_local_job(frame, area, label, tick, focused);
            return;
        }
        if let Some(id) = self.pending_id {
            self.render_pending(frame, area, id, tick, focused);
            return;
        }
        if !self.results.is_empty() {
            self.render_done(frame, area, focused);
            return;
        }
        self.render_idle(frame, area, focused);
    }

    fn panel_border(focused: bool, idle: Color) -> Style {
        if focused {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(idle)
        }
    }

    fn render_idle(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let block = Block::default()
            .title(if focused { "Output (focused)" } else { "Output" })
            .borders(Borders::ALL)
            .border_style(Self::panel_border(focused, Color::DarkGray));
        let hint = if focused {
            format!("{OUTPUT_IDLE_HINT}\n{OUTPUT_FOCUS_HINT}")
        } else {
            OUTPUT_IDLE_HINT.into()
        };
        frame.render_widget(
            Paragraph::new(hint)
                .block(block)
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
    }

    fn render_error(&self, frame: &mut Frame, area: Rect, err: &str, focused: bool) {
        let block = Block::default()
            .title(if focused {
                "Output (error, focused)"
            } else {
                "Output (error)"
            })
            .borders(Borders::ALL)
            .border_style(Self::panel_border(focused, Color::Red));
        let body = if focused {
            format!("✗ {err}\nFix and press F5 — Esc returns to form")
        } else {
            format!("✗ {err}\nFix and press F5 to retry")
        };
        frame.render_widget(Paragraph::new(body).block(block), area);
    }

    fn working_border(tick: u64, pulse: u64) -> Style {
        let phase = (tick.wrapping_add(pulse)).rem_euclid(4);
        let color = match phase {
            0 => Color::Yellow,
            1 => Color::LightYellow,
            2 => Color::Cyan,
            _ => Color::LightCyan,
        };
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    }

    fn render_local_job(&self, frame: &mut Frame, area: Rect, label: &str, tick: u64, focused: bool) {
        let icon = status_icon(super::task_display::TaskStatusKind::Running, tick);
        let dots = ".".repeat(((tick / 2) % 4) as usize + 1);
        let body = format!("{icon} SUBMITTED — {label}{dots}\nIn progress — Tab to switch section");
        let block = Block::default()
            .title(if focused {
                "Output (working, focused)"
            } else {
                "Output (working)"
            })
            .borders(Borders::ALL)
            .border_style(if focused {
                Self::working_border(tick, self.pulse)
            } else {
                Style::default().fg(Color::Yellow)
            });
        frame.render_widget(Paragraph::new(body).block(block), area);
    }

    fn render_done(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let labels: Vec<&str> = RESULT_FIELDS.to_vec();
        let tabs = detail_field_tabs_line(&labels, self.field);
        let title = if self.results.len() > 1 {
            format!("Output — result {}/{}", self.index + 1, self.results.len())
        } else if focused {
            "Output — done (focused)".into()
        } else {
            "Output — done".into()
        };
        let value = format!(
            "{}\n{}",
            detail_value_display(&self.field_value(self.field)),
            if focused { OUTPUT_FOCUS_HINT } else { OUTPUT_DONE_HINT }
        );
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Self::panel_border(focused, Color::Green));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(inner);

        frame.render_widget(ratatui::widgets::Paragraph::new(tabs), chunks[0]);
        frame.render_widget(
            ratatui::widgets::Paragraph::new(value).style(Style::default().fg(Color::White)),
            chunks[1],
        );
    }

    fn render_pending(&self, frame: &mut Frame, area: Rect, local_id: i64, tick: u64, focused: bool) {
        let snapshot = VideoTaskRecord {
            id: local_id,
            task_id: String::new(),
            status: self.pending_status.clone(),
            phase: "processing".into(),
            prompt: None,
            input_json: None,
            progress: self.pending_progress,
            uri: None,
            asset_uri: None,
            error: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let kind = task_status_kind(&snapshot);
        let icon = status_icon(kind, tick);
        let dots = ".".repeat(((tick / 2) % 4) as usize + 1);
        let body = format!(
            "{icon} SUBMITTED — Task #{local_id} {} · {} {}{dots}\nAsync video running — URI fills here when complete",
            self.pending_status,
            progress_bar(self.pending_progress, 12),
            progress_percent(self.pending_progress)
        );
        let block = Block::default()
            .title(if focused {
                "Output (task running, focused)"
            } else {
                "Output (task running)"
            })
            .borders(Borders::ALL)
            .border_style(if focused {
                Self::working_border(tick, self.pulse)
            } else {
                Style::default().fg(Color::Cyan)
            });
        frame.render_widget(Paragraph::new(body).block(block), area);
    }
}
