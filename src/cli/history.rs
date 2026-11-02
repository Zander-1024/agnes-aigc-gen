use anyhow::Result;
use clap::{Args, Subcommand};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, Widget};
use std::io::{self, IsTerminal};

use crate::cli::list_tui::{
    LIST_HELP_TEXT, ListTable, buffer_to_string, run_list_tui, split_list_layout, truncate_display,
};
use crate::cli::util::non_empty_value;
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
    /// List recent HTTP request parameter snapshots (including failed requests)
    Requests {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show effective parameters and configuration fingerprint for a request
    Request { id: i64 },
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
        HistoryAction::Requests { limit } => {
            println!("{}", serde_json::to_string_pretty(&db.list_request_runs(limit)?)?)
        }
        HistoryAction::Request { id } => println!("{}", serde_json::to_string_pretty(&db.get_request_run(id)?)?),
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

const ASSET_DETAIL_LABELS: [&str; 3] = ["ASSET URI", "REMOTE URL", "ID"];
const ASSET_DETAIL_FIELD_COUNT: usize = ASSET_DETAIL_LABELS.len();

fn asset_detail_value(row: &AssetRecord, field: usize) -> Option<&str> {
    match field {
        0 => non_empty_value(Some(row.asset_uri.as_str())),
        1 => non_empty_value(Some(row.remote_url.as_str())),
        2 => non_empty_value(Some(row.id.as_str())),
        _ => None,
    }
}

fn run_asset_list_tui(rows: Vec<AssetRecord>) -> Result<()> {
    run_list_tui(
        rows,
        "No assets recorded.",
        ASSET_DETAIL_FIELD_COUNT,
        asset_detail_value,
        render_asset_list_ui,
    )
}

fn render_asset_list_ui(frame: &mut ratatui::Frame, state: &mut ListTable<AssetRecord>) {
    let chunks = split_list_layout(frame.area());

    let table = asset_table(&state.rows)
        .block(Block::default().title("Assets").borders(Borders::ALL))
        .highlight_symbol(">> ")
        .row_highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_stateful_widget(table, chunks.table, state.table_state_mut());

    crate::cli::list_tui::render_list_detail_help(
        frame,
        chunks.detail,
        chunks.help,
        &ASSET_DETAIL_LABELS,
        state.detail_field,
        state.selected_detail_display(asset_detail_value),
        LIST_HELP_TEXT,
    );
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
        let mut state = ListTable::new(vec![sample_asset()]);

        assert_eq!(state.detail_field, 0);
        assert_eq!(
            state.selected_detail_copy_value(asset_detail_value),
            Some("asset://c8d4eb63a84b")
        );

        state.select_next_detail(ASSET_DETAIL_FIELD_COUNT);
        assert_eq!(state.detail_field, 1);
        assert_eq!(
            state.selected_detail_copy_value(asset_detail_value),
            Some("https://cdn.example.com/images/abc.png")
        );

        state.select_next_detail(ASSET_DETAIL_FIELD_COUNT);
        assert_eq!(state.detail_field, 2);
        assert_eq!(
            state.selected_detail_copy_value(asset_detail_value),
            Some("c8d4eb63a84b")
        );
    }

    #[test]
    fn asset_detail_display_truncates_long_remote_url() {
        let mut asset = sample_asset();
        asset.remote_url = "https://example.com/".to_string() + &"x".repeat(250);

        let state = ListTable::with_detail_field(vec![asset], 1);
        let displayed = state.selected_detail_display(asset_detail_value);
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
