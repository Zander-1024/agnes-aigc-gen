use anyhow::Result;
use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};

use crate::api::{ApiClient, refresh_video_task};
use crate::cli::list_tui::{
    ListTable, copy_to_clipboard_silent, parse_list_key, render_list_detail_help, split_list_layout, truncate_display,
};
use crate::cli::util::non_empty_value;
use crate::config::AppConfig;
use crate::db::{Database, VideoTaskRecord};

use super::task_display::{
    TaskStripData, build_task_strip, progress_bar, progress_percent, status_color, status_icon, status_label,
    task_status_kind,
};

const TASK_HELP: &str = "↑↓ row  ←→ field  Enter copy  r refresh  g running  Esc back";
const TASK_DETAIL_LABELS: [&str; 3] = ["QUERY ID", "PROMPT", "URI"];
const TASK_DETAIL_FIELD_COUNT: usize = TASK_DETAIL_LABELS.len();

fn task_detail_value(row: &VideoTaskRecord, field: usize) -> Option<&str> {
    match field {
        0 => non_empty_value(Some(row.task_id.as_str())),
        1 => non_empty_value(row.prompt.as_deref()),
        2 => non_empty_value(row.uri.as_deref()),
        _ => None,
    }
}

pub struct TasksView {
    pub list: ListTable<VideoTaskRecord>,
    pub highlight_id: Option<i64>,
}

impl TasksView {
    pub fn load(limit: usize) -> Result<Self> {
        Ok(Self { list: ListTable::new(refresh_tasks(limit)?), highlight_id: None })
    }

    pub fn refresh(&mut self, limit: usize) {
        if let Ok(rows) = refresh_tasks(limit) {
            self.list.rows = rows;
            if let Some(id) = self.highlight_id {
                if let Some(index) = self.list.rows.iter().position(|r| r.id == id) {
                    self.list.table_state_mut().select(Some(index));
                }
            } else if self.list.selected_index().is_none() && !self.list.rows.is_empty() {
                self.list.table_state_mut().select(Some(0));
            }
        }
    }

    pub fn select_task_id(&mut self, local_id: i64) {
        self.highlight_id = Some(local_id);
        if let Some(index) = self.list.rows.iter().position(|r| r.id == local_id) {
            self.list.table_state_mut().select(Some(index));
        }
    }

    pub fn select_primary_running(&mut self) -> bool {
        let id = self.primary_running().map(|r| r.id);
        if let Some(id) = id {
            self.select_task_id(id);
            true
        } else {
            false
        }
    }

    pub fn processing_count(&self) -> usize {
        self.list.rows.iter().filter(|r| r.phase == "processing").count()
    }

    pub fn has_processing(&self) -> bool {
        self.processing_count() > 0
    }

    pub fn row_by_id(&self, local_id: i64) -> Option<&VideoTaskRecord> {
        self.list.rows.iter().find(|r| r.id == local_id)
    }

    pub fn primary_running(&self) -> Option<&VideoTaskRecord> {
        self.list
            .rows
            .iter()
            .filter(|r| r.phase == "processing")
            .max_by_key(|r| (r.progress.unwrap_or(0), r.id))
    }

    pub fn task_strip_data(&self, tick: u64) -> TaskStripData {
        build_task_strip(&self.list.rows, tick, 2, 2)
    }

    pub fn handle_key(
        &mut self,
        key: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> TaskAction {
        if key == crossterm::event::KeyCode::Char('r') {
            return TaskAction::Refresh;
        }
        if key == crossterm::event::KeyCode::Char('g') {
            return TaskAction::SelectRunning;
        }

        let action = parse_list_key(key, modifiers);
        if action.quit {
            return TaskAction::Back;
        }
        self.list.apply_key_action(&action, TASK_DETAIL_FIELD_COUNT);
        if action.copy {
            copy_to_clipboard_silent(self.list.selected_detail_copy_value(task_detail_value).unwrap_or("-"));
        }
        TaskAction::None
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, tick: u64) {
        let chunks = split_list_layout(area);

        let running = self.processing_count();
        let title = if running > 0 {
            format!("Video Tasks ({running} running)")
        } else {
            "Video Tasks".into()
        };

        let table = task_table(&self.list.rows, tick)
            .block(Block::default().title(title).borders(Borders::ALL))
            .highlight_symbol(">> ")
            .row_highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
        frame.render_stateful_widget(table, chunks.table, self.list.table_state_mut());

        render_list_detail_help(
            frame,
            chunks.detail,
            chunks.help,
            &TASK_DETAIL_LABELS,
            self.list.detail_field,
            self.list.selected_detail_display(task_detail_value),
            TASK_HELP,
        );
    }
}

pub enum TaskAction {
    None,
    Back,
    Refresh,
    SelectRunning,
}

fn refresh_tasks(limit: usize) -> Result<Vec<VideoTaskRecord>> {
    let db = Database::open()?;
    let pending = db.list_video_tasks(limit)?;
    if let Ok(api) = ApiClient::from_config(AppConfig::load()?) {
        for row in pending.iter().filter(|r| r.phase == "processing") {
            if let Err(err) = refresh_video_task(&api, &row.task_id) {
                log::warn!("refresh task {} (#{}): {err}", row.task_id, row.id);
            }
        }
    }
    db.list_video_tasks(limit)
}

fn task_table(rows: &[VideoTaskRecord], tick: u64) -> Table<'static> {
    let header = Row::new([
        Cell::from("ID"),
        Cell::from("STATE"),
        Cell::from("STATUS"),
        Cell::from("PROGRESS"),
        Cell::from("PROMPT"),
        Cell::from("URI"),
    ])
    .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
    .height(1);

    Table::new(
        rows.iter().map(|row| task_row_cells(row, tick)),
        [
            Constraint::Length(5),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(22),
            Constraint::Length(20),
            Constraint::Min(12),
        ],
    )
    .header(header)
    .column_spacing(1)
}

fn task_row_cells(row: &VideoTaskRecord, tick: u64) -> Row<'static> {
    let kind = task_status_kind(row);
    let color = status_color(kind);
    let state_style = Style::default().fg(color);
    let icon = status_icon(kind, tick);
    let state = format!("{icon} {}", status_label(kind));
    let progress = format!("{} {}", progress_bar(row.progress, 10), progress_percent(row.progress));
    Row::new([
        Cell::from(row.id.to_string()),
        Cell::from(Line::from(state)).style(state_style),
        Cell::from(row.status.clone()),
        Cell::from(progress),
        Cell::from(truncate_display(&row.prompt.clone().unwrap_or_default(), 18)),
        Cell::from(truncate_display(row.uri.as_deref().unwrap_or("-"), 28)),
    ])
}
