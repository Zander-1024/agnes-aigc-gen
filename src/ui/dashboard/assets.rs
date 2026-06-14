use anyhow::Result;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};

use crate::cli::list_tui::{
    copy_to_clipboard_silent, detail_field_tabs_line, detail_value_display, parse_list_key, render_detail_panel,
    truncate_display,
};
use crate::db::{AssetRecord, Database};

const ASSET_HELP: &str = "↑↓ row  ←→ field  Enter copy  v use for video  r refresh  Esc back";

pub struct AssetsView {
    pub rows: Vec<AssetRecord>,
    pub table_state: TableState,
    pub detail_field: usize,
}

impl AssetsView {
    pub fn load(limit: usize) -> Result<Self> {
        let db = Database::open()?;
        let rows = db.list_assets(limit)?;
        let mut table_state = TableState::default();
        if !rows.is_empty() {
            table_state.select(Some(0));
        }
        Ok(Self { rows, table_state, detail_field: 0 })
    }

    pub fn refresh(&mut self, limit: usize) {
        if let Ok(db) = Database::open()
            && let Ok(rows) = db.list_assets(limit)
        {
            self.rows = rows;
            if self.table_state.selected().is_none() && !self.rows.is_empty() {
                self.table_state.select(Some(0));
            }
        }
    }

    pub fn selected_row(&self) -> Option<&AssetRecord> {
        self.table_state.selected().and_then(|i| self.rows.get(i))
    }

    pub fn selected_asset_uri(&self) -> Option<String> {
        self.selected_row().map(|r| r.asset_uri.clone())
    }

    fn detail_labels() -> [&'static str; 3] {
        ["ASSET URI", "REMOTE URL", "ID"]
    }

    fn detail_value(&self) -> &str {
        let row = match self.selected_row() {
            Some(r) => r,
            None => return "-",
        };
        match self.detail_field {
            0 => row.asset_uri.as_str(),
            1 => row.remote_url.as_str(),
            _ => row.id.as_str(),
        }
    }

    pub fn handle_key(
        &mut self,
        key: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> AssetAction {
        let action = parse_list_key(key, modifiers);
        if action.quit {
            return AssetAction::Back;
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
            return AssetAction::Refresh;
        }
        if key == crossterm::event::KeyCode::Char('v') {
            return AssetAction::UseForVideo;
        }
        AssetAction::None
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

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(4), Constraint::Length(1)])
            .split(area);

        let table = asset_table(&self.rows)
            .block(Block::default().title("Assets").borders(Borders::ALL))
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
            ratatui::widgets::Paragraph::new(ASSET_HELP).style(Style::default().fg(Color::DarkGray)),
            chunks[2],
        );
    }
}

pub enum AssetAction {
    None,
    Back,
    Refresh,
    UseForVideo,
}

fn asset_table(rows: &[AssetRecord]) -> Table<'static> {
    let header = Row::new([Cell::from("ASSET URI"), Cell::from("KIND"), Cell::from("RATIO"), Cell::from("REMOTE URL")])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .height(1);

    Table::new(
        rows.iter().map(|row| {
            Row::new([
                Cell::from(truncate_display(&row.asset_uri, 22)),
                Cell::from(row.kind.clone()),
                Cell::from(row.ratio.clone().unwrap_or_else(|| "-".into())),
                Cell::from(truncate_display(&row.remote_url, 48)),
            ])
        }),
        [Constraint::Length(24), Constraint::Length(8), Constraint::Length(8), Constraint::Min(20)],
    )
    .header(header)
    .column_spacing(1)
}
