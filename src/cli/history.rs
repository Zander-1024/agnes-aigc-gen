use anyhow::Result;
use clap::{Args, Subcommand};
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState, Widget};
use std::io::{self, IsTerminal};
use std::time::Duration;

use crate::cli::list_tui::{
    LIST_HELP_TEXT, buffer_to_string, copy_to_clipboard_silent, detail_field_tabs_line, detail_value_display,
    parse_list_key, render_detail_panel, render_help_line, truncate_display,
};
use crate::db::{AssetRecord, Database};

#[derive(Args)]
pub struct HistoryCmd {
    #[command(subcommand)]
    pub action: HistoryAction,
}

#[derive(Subcommand)]
pub enum HistoryAction {
    /// List recent generations
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show a generation record by id
    Show { id: i64 },
}

#[derive(Args)]
pub struct AssetCmd {
    #[command(subcommand)]
    pub action: AssetAction,
}

#[derive(Subcommand)]
pub enum AssetAction {
    /// List assets in the library
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long = "output-format", default_value = "plain")]
        output_format: String,
    },
    /// Show an asset (accepts asset://id or bare id)
    Show { id: String },
}

pub fn run_history(cmd: HistoryCmd) -> Result<()> {
    let db = Database::open()?;
    match cmd.action {
        HistoryAction::List { limit } => {
            let rows = db.list_generations(limit)?;
            if rows.is_empty() {
                println!("No generations recorded.");
                return Ok(());
            }
            for row in rows {
                let prompt = row.prompt.unwrap_or_default();
                let prompt_short = if prompt.chars().count() > 40 {
                    format!("{}…", prompt.chars().take(39).collect::<String>())
                } else {
                    prompt
                };
                println!(
                    "#{:<6} {:<6} {:<20} asset={}",
                    row.id,
                    row.kind,
                    prompt_short,
                    row.asset_id.unwrap_or_else(|| "-".into())
                );
            }
        }
        HistoryAction::Show { id } => {
            let row = db.get_generation(id)?;
            println!("{}", serde_json::to_string_pretty(&row)?);
        }
    }
    Ok(())
}

pub fn run_asset(cmd: AssetCmd) -> Result<()> {
    let db = Database::open()?;
    match cmd.action {
        AssetAction::List { limit, output_format } => run_asset_list(limit, output_format, &db),
        AssetAction::Show { id } => {
            let asset = db.get_asset(&id)?;
            println!("{}", serde_json::to_string_pretty(&asset)?);
            Ok(())
        }
    }
}

fn run_asset_list(limit: usize, output_format: String, db: &Database) -> Result<()> {
    let rows = db.list_assets(limit)?;
    match output_format.to_lowercase().as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&rows)?),
        "plain" => {
            if io::stdout().is_terminal() {
                run_asset_list_tui(rows)?;
            } else {
                print_asset_list_table(&rows);
            }
        }
        "table" => print_asset_list_table(&rows),
        other => anyhow::bail!("unknown output format: {other}"),
    }
    Ok(())
}

fn print_asset_list_table(rows: &[AssetRecord]) {
    if rows.is_empty() {
        println!("No assets recorded.");
        return;
    }
    println!("{}", render_asset_list_table(rows));
}

struct AssetListUiState {
    rows: Vec<AssetRecord>,
    table_state: TableState,
    detail_field: AssetDetailField,
}

impl AssetListUiState {
    fn new(rows: Vec<AssetRecord>) -> Self {
        let mut table_state = TableState::default();
        if !rows.is_empty() {
            table_state.select(Some(0));
        }
        Self { rows, table_state, detail_field: AssetDetailField::AssetUri }
    }

    fn selected_index(&self) -> Option<usize> {
        self.table_state.selected().filter(|index| *index < self.rows.len())
    }

    fn selected_row(&self) -> Option<&AssetRecord> {
        self.selected_index().and_then(|index| self.rows.get(index))
    }

    fn selected_detail_value(&self) -> Option<&str> {
        let row = self.selected_row()?;
        match self.detail_field {
            AssetDetailField::AssetUri => non_empty_value(Some(row.asset_uri.as_str())),
            AssetDetailField::RemoteUrl => non_empty_value(Some(row.remote_url.as_str())),
            AssetDetailField::Id => non_empty_value(Some(row.id.as_str())),
        }
    }

    fn selected_detail_display_value(&self) -> String {
        detail_value_display(self.selected_detail_value().unwrap_or("-"))
    }

    fn select_next(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        let next = self
            .selected_index()
            .map_or(0, |index| index.saturating_add(1).min(self.rows.len() - 1));
        self.table_state.select(Some(next));
    }

    fn select_previous(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        let previous = self.selected_index().map_or(0, |index| index.saturating_sub(1));
        self.table_state.select(Some(previous));
    }

    fn select_first(&mut self) {
        if !self.rows.is_empty() {
            self.table_state.select(Some(0));
        }
    }

    fn select_last(&mut self) {
        if !self.rows.is_empty() {
            self.table_state.select(Some(self.rows.len() - 1));
        }
    }

    fn select_next_detail(&mut self) {
        self.detail_field = self.detail_field.next();
    }

    fn select_previous_detail(&mut self) {
        self.detail_field = self.detail_field.previous();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetDetailField {
    AssetUri,
    RemoteUrl,
    Id,
}

impl AssetDetailField {
    const ALL: [Self; 3] = [Self::AssetUri, Self::RemoteUrl, Self::Id];

    fn label(self) -> &'static str {
        match self {
            Self::AssetUri => "ASSET URI",
            Self::RemoteUrl => "REMOTE URL",
            Self::Id => "ID",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::AssetUri => 0,
            Self::RemoteUrl => 1,
            Self::Id => 2,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::AssetUri => Self::RemoteUrl,
            Self::RemoteUrl => Self::Id,
            Self::Id => Self::AssetUri,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::AssetUri => Self::Id,
            Self::RemoteUrl => Self::AssetUri,
            Self::Id => Self::RemoteUrl,
        }
    }
}

fn run_asset_list_tui(rows: Vec<AssetRecord>) -> Result<()> {
    if rows.is_empty() {
        println!("No assets recorded.");
        return Ok(());
    }

    let mut terminal = ratatui::try_init()?;
    let result = run_asset_list_app(&mut terminal, rows);
    ratatui::restore();
    result
}

fn run_asset_list_app(terminal: &mut DefaultTerminal, rows: Vec<AssetRecord>) -> Result<()> {
    let mut state = AssetListUiState::new(rows);
    loop {
        terminal.draw(|frame| render_asset_list_ui(frame, &mut state))?;
        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if handle_asset_list_key(key.code, key.modifiers, &mut state) {
                break;
            }
        }
    }
    Ok(())
}

fn handle_asset_list_key(
    key: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
    state: &mut AssetListUiState,
) -> bool {
    let action = parse_list_key(key, modifiers);
    if action.quit {
        return true;
    }
    if action.row_up {
        state.select_previous();
    }
    if action.row_down {
        state.select_next();
    }
    if action.row_first {
        state.select_first();
    }
    if action.row_last {
        state.select_last();
    }
    if action.row_page_up {
        for _ in 0..10 {
            state.select_previous();
        }
    }
    if action.row_page_down {
        for _ in 0..10 {
            state.select_next();
        }
    }
    if action.field_previous {
        state.select_previous_detail();
    }
    if action.field_next {
        state.select_next_detail();
    }
    if action.copy
        && let Some(value) = state.selected_detail_value()
    {
        copy_to_clipboard_silent(value);
    }
    false
}

fn render_asset_list_ui(frame: &mut ratatui::Frame, state: &mut AssetListUiState) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(4), Constraint::Length(1)])
        .split(area);

    let table = asset_table(&state.rows)
        .block(Block::default().title("Assets").borders(Borders::ALL))
        .highlight_symbol(">> ")
        .row_highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_stateful_widget(table, chunks[0], &mut state.table_state);

    let labels: Vec<&str> = AssetDetailField::ALL.iter().map(|field| field.label()).collect();
    let tabs = detail_field_tabs_line(&labels, state.detail_field.index());
    let value = state.selected_detail_display_value();
    render_detail_panel(frame, chunks[1], tabs, value);

    render_help_line(frame, chunks[2], LIST_HELP_TEXT);
}

fn render_asset_list_table(rows: &[AssetRecord]) -> String {
    let width = crossterm::terminal::size()
        .map(|(width, _)| width.clamp(120, 180))
        .unwrap_or(140);
    render_asset_list_table_with_width(rows, width)
}

fn render_asset_list_table_with_width(rows: &[AssetRecord], width: u16) -> String {
    let height = rows.len().saturating_add(3).min(u16::MAX as usize) as u16;
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);

    let table = asset_table(rows).block(Block::default().title("Assets").borders(Borders::ALL));

    table.render(area, &mut buffer);
    buffer_to_string(&buffer)
}

fn asset_table(rows: &[AssetRecord]) -> Table<'static> {
    let header = Row::new([Cell::from("ASSET URI"), Cell::from("KIND"), Cell::from("RATIO"), Cell::from("REMOTE URL")])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .height(1);

    Table::new(
        rows.iter().map(asset_table_row),
        [Constraint::Length(24), Constraint::Length(8), Constraint::Length(8), Constraint::Min(20)],
    )
    .header(header)
    .column_spacing(1)
}

fn asset_table_row(row: &AssetRecord) -> Row<'static> {
    Row::new([
        Cell::from(truncate_display(&row.asset_uri, 22)),
        Cell::from(row.kind.clone()),
        Cell::from(row.ratio.clone().unwrap_or_else(|| "-".into())),
        Cell::from(truncate_display(&row.remote_url, 64)),
    ])
}

fn non_empty_value(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() { None } else { Some(trimmed) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_table_renders_columns() {
        let rows = vec![sample_asset()];
        let table = render_asset_list_table_with_width(&rows, 120);

        assert!(table.contains("Assets"));
        assert!(table.contains("ASSET URI"));
        assert!(table.contains("asset://c8d4eb63"));
    }

    #[test]
    fn asset_list_detail_field_cycles() {
        let mut state = AssetListUiState::new(vec![sample_asset()]);

        assert_eq!(state.detail_field, AssetDetailField::AssetUri);
        assert_eq!(state.selected_detail_value(), Some("asset://c8d4eb63a84b"));

        state.select_next_detail();
        assert_eq!(state.detail_field, AssetDetailField::RemoteUrl);
        assert_eq!(
            state.selected_detail_value(),
            Some("https://cdn.example.com/images/abc.png")
        );

        state.select_next_detail();
        assert_eq!(state.detail_field, AssetDetailField::Id);
        assert_eq!(state.selected_detail_value(), Some("c8d4eb63a84b"));
    }

    #[test]
    fn asset_detail_display_truncates_long_remote_url() {
        let mut asset = sample_asset();
        asset.remote_url = "https://example.com/".to_string() + &"x".repeat(250);

        let mut state = AssetListUiState::new(vec![asset]);
        state.detail_field = AssetDetailField::RemoteUrl;
        let displayed = state.selected_detail_display_value();
        assert_eq!(
            displayed.chars().count(),
            crate::cli::list_tui::DETAIL_DISPLAY_MAX_CHARS
        );
        assert!(displayed.ends_with("..."));
    }

    fn sample_asset() -> AssetRecord {
        AssetRecord {
            id: "c8d4eb63a84b".into(),
            asset_uri: "asset://c8d4eb63a84b".into(),
            kind: "image".into(),
            remote_url: "https://cdn.example.com/images/abc.png".into(),
            ratio: Some("16:9".into()),
            size: None,
            created_at: "2026-06-06T00:00:00Z".into(),
        }
    }
}
