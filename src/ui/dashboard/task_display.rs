use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::cli::list_tui::truncate_display;
use crate::db::VideoTaskRecord;

pub const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskStatusKind {
    Queued,
    Running,
    Completed,
    Failed,
}

pub fn task_status_kind(row: &VideoTaskRecord) -> TaskStatusKind {
    match row.status.as_str() {
        "completed" => TaskStatusKind::Completed,
        "failed" => TaskStatusKind::Failed,
        "queued" => TaskStatusKind::Queued,
        _ => TaskStatusKind::Running,
    }
}

pub fn status_icon(kind: TaskStatusKind, tick: u64) -> &'static str {
    match kind {
        TaskStatusKind::Queued => "○",
        TaskStatusKind::Running => SPINNER[(tick as usize / 3) % SPINNER.len()],
        TaskStatusKind::Completed => "✓",
        TaskStatusKind::Failed => "✗",
    }
}

pub fn status_label(kind: TaskStatusKind) -> &'static str {
    match kind {
        TaskStatusKind::Queued => "queued",
        TaskStatusKind::Running => "running",
        TaskStatusKind::Completed => "done",
        TaskStatusKind::Failed => "failed",
    }
}

pub fn status_color(kind: TaskStatusKind) -> Color {
    match kind {
        TaskStatusKind::Queued => Color::DarkGray,
        TaskStatusKind::Running => Color::Cyan,
        TaskStatusKind::Completed => Color::Green,
        TaskStatusKind::Failed => Color::Red,
    }
}

pub fn progress_percent(progress: Option<i32>) -> String {
    match progress {
        Some(p) => format!("{}%", p.clamp(0, 100)),
        None => "-".into(),
    }
}

pub fn progress_bar(progress: Option<i32>, width: usize) -> String {
    let width = width.max(4);
    let pct = progress.map(|p| p.clamp(0, 100) as usize).unwrap_or(0);
    let filled = pct * width / 100;
    let empty = width.saturating_sub(filled);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

pub fn task_row_line(row: &VideoTaskRecord, tick: u64, prompt_width: usize) -> Line<'static> {
    let kind = task_status_kind(row);
    let color = status_color(kind);
    let icon = status_icon(kind, tick);
    let prompt = truncate_display(&row.prompt.clone().unwrap_or_default(), prompt_width);
    Line::from(vec![
        Span::styled(format!("{icon} "), Style::default().fg(color)),
        Span::raw(format!("#{} ", row.id)),
        Span::styled(format!("{} ", status_label(kind)), Style::default().fg(color)),
        Span::raw(format!(
            "{} {} ",
            progress_bar(row.progress, 8),
            progress_percent(row.progress)
        )),
        Span::styled(prompt, Style::default().fg(Color::DarkGray)),
    ])
}

pub struct TaskStripData {
    pub active: Vec<Line<'static>>,
    pub recent: Vec<Line<'static>>,
}

pub fn build_task_strip(
    rows: &[VideoTaskRecord],
    tick: u64,
    active_limit: usize,
    recent_limit: usize,
) -> TaskStripData {
    let active: Vec<Line<'static>> = rows
        .iter()
        .filter(|r| r.phase == "processing")
        .take(active_limit)
        .map(|r| task_row_line(r, tick, 14))
        .collect();
    let recent: Vec<Line<'static>> = rows
        .iter()
        .filter(|r| r.phase != "processing")
        .take(recent_limit)
        .map(|r| task_row_line(r, tick, 14))
        .collect();
    TaskStripData { active, recent }
}

pub fn generation_result_from_task(row: &VideoTaskRecord) -> Option<crate::output::GenerationResult> {
    if row.phase != "success" {
        return None;
    }
    Some(crate::output::GenerationResult {
        kind: "video".into(),
        ratio: "-".into(),
        size: "-".into(),
        uri: row.uri.clone().unwrap_or_default(),
        asset_uri: row.asset_uri.clone(),
        generation_id: Some(row.id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row(status: &str, phase: &str, progress: Option<i32>) -> VideoTaskRecord {
        VideoTaskRecord {
            id: 1,
            task_id: "t1".into(),
            status: status.into(),
            phase: phase.into(),
            prompt: Some("test".into()),
            input_json: None,
            progress,
            uri: None,
            asset_uri: None,
            error: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn maps_queued_status() {
        assert_eq!(
            task_status_kind(&sample_row("queued", "processing", None)),
            TaskStatusKind::Queued
        );
    }

    #[test]
    fn maps_in_progress_status() {
        assert_eq!(
            task_status_kind(&sample_row("in_progress", "processing", Some(40))),
            TaskStatusKind::Running
        );
    }

    #[test]
    fn progress_bar_fills_by_percent() {
        let bar = progress_bar(Some(50), 10);
        assert!(bar.contains("█████"));
        assert!(bar.contains("░░░░░"));
    }
}
