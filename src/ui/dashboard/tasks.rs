use anyhow::Result;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};

use crate::api::{ApiClient, refresh_video_task};
use crate::cli::list_tui::{
    copy_to_clipboard_silent, detail_field_tabs_line, detail_value_display, parse_list_key, render_detail_panel,
    truncate_display,
};
use crate::config::AppConfig;
use crate::db::{Database, VideoTaskRecord};

use super::task_display::{
    TaskStripData, build_task_strip, progress_bar, progress_percent, status_color, status_icon, status_label,
    task_status_kind,
};

const TASK_HELP: &str = "↑↓ row  ←→ field  Enter copy  r refresh  g running  Esc back";

pub struct TasksView {
    pub rows: Vec<VideoTaskRecord>,
    pub table_state: TableState,
    pub detail_field: usize,
    pub highlight_id: Option<i64>,
}

impl TasksView {
    pub fn load(limit: usize) -> Result<Self> {
        Ok(Self {
            rows: refresh_tasks(limit)?,
            table_state: TableState::default(),
            detail_field: 0,
            highlight_id: None,
        })
    }

    pub fn refresh(&mut self, limit: usize) {
        if let Ok(rows) = refresh_tasks(limit) {
            self.rows = rows;
            if let Some(id) = self.highlight_id {
                if let Some(index) = self.rows.iter().position(|r| r.id == id) {
                    self.table_state.select(Some(index));
                }
            } else if self.table_state.selected().is_none() && !self.rows.is_empty() {
                self.table_state.select(Some(0));
            }
        }
    }

    pub fn select_task_id(&mut self, local_id: i64) {
        self.highlight_id = Some(local_id);
        if let Some(index) = self.rows.iter().position(|r| r.id == local_id) {
            self.table_state.select(Some(index));
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
        self.rows.iter().filter(|r| r.phase == "processing").count()
    }

    pub fn has_processing(&self) -> bool {
        self.processing_count() > 0
    }

    pub fn row_by_id(&self, local_id: i64) -> Option<&VideoTaskRecord> {
        self.rows.iter().find(|r| r.id == local_id)
    }

    pub fn primary_running(&self) -> Option<&VideoTaskRecord> {
        self.rows
            .iter()
            .filter(|r| r.phase == "processing")
            .max_by_key(|r| (r.progress.unwrap_or(0), r.id))
    }

    pub fn task_strip_data(&self, tick: u64) -> TaskStripData {
        build_task_strip(&self.rows, tick, 2, 2)
    }

    pub fn selected_row(&self) -> Option<&VideoTaskRecord> {
        self.table_state.selected().and_then(|i| self.rows.get(i))
    }

    fn detail_labels() -> [&'static str; 3] {
        ["QUERY ID", "PROMPT", "URI"]
    }

    fn detail_value(&self) -> &str {
        let row = match self.selected_row() {
            Some(r) => r,
            None => return "-",
        };
        match self.detail_field {
            0 => row.task_id.as_str(),
            1 => row.prompt.as_deref().unwrap_or("-"),
            _ => row.uri.as_deref().unwrap_or("-"),
        }
    }

    pub fn handle_key(
        &mut self,
        key: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> TaskAction {
        let action = parse_list_key(key, modifiers);
        if action.quit {
            return TaskAction::Back;
        }
        if action.row_up {
            self.select_previous();
        }
        if action.row_down {
            self.select_next();
        }
        if action.field_previous {
            self.detail_field = if self.detail_field == 0 {
                2
            } else {
                self.detail_field - 1
            };
        }
        if action.field_next {
            self.detail_field = (self.detail_field + 1) % 3;
        }
        if action.copy {
            copy_to_clipboard_silent(self.detail_value());
        }
        if key == crossterm::event::KeyCode::Char('r') {
            return TaskAction::Refresh;
        }
        if key == crossterm::event::KeyCode::Char('g') {
            return TaskAction::SelectRunning;
        }
        TaskAction::None
    }

    fn select_next(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        let next = self
            .table_state
            .selected()
            .map_or(0, |i| i.saturating_add(1).min(self.rows.len() - 1));
        self.table_state.select(Some(next));
    }

    fn select_previous(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        let prev = self.table_state.selected().map_or(0, |i| i.saturating_sub(1));
        self.table_state.select(Some(prev));
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, tick: u64) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(4), Constraint::Length(1)])
            .split(area);

        let running = self.processing_count();
        let title = if running > 0 {
            format!("Video Tasks ({running} running)")
        } else {
            "Video Tasks".into()
        };

        let table = task_table(&self.rows, tick)
            .block(Block::default().title(title).borders(Borders::ALL))
            .highlight_symbol(">> ")
            .row_highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
        frame.render_stateful_widget(table, chunks[0], &mut self.table_state);

        let labels: Vec<&str> = Self::detail_labels().to_vec();
        let tabs = detail_field_tabs_line(&labels, self.detail_field);
        let value = detail_value_display(self.detail_value());
        render_detail_panel(frame, chunks[1], tabs, value);

        frame.render_widget(
            ratatui::widgets::Paragraph::new(TASK_HELP).style(Style::default().fg(Color::DarkGray)),
            chunks[2],
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
