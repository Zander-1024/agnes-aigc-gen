use anyhow::Result;
use clap::{Args, Subcommand};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, Widget};

use crate::api::{ApiClient, refresh_video_task, wait_video_task};
use crate::config::AppConfig;
use crate::db::{Database, VideoTaskRecord};
use crate::output::OutputFormat;

#[derive(Args)]
pub struct TaskCmd {
    #[command(subcommand)]
    pub action: TaskAction,
}

#[derive(Subcommand)]
pub enum TaskAction {
    /// List recent video tasks (default 10); refreshes in-progress tasks from API
    List {
        #[arg(short = 'n', long = "limit", default_value_t = 10)]
        limit: usize,
        #[arg(long = "output-format", default_value = "plain")]
        output_format: String,
    },
    /// Query a video task once (refresh from API, update local record)
    Show {
        /// Local id (e.g. `3`), `#3`, or vendor task id
        task_ref: String,
        #[arg(long = "output-format", default_value = "json")]
        output_format: String,
    },
    /// Poll until a video task completes (same as `video --task-id`)
    Wait {
        /// Local id (e.g. `3`), `#3`, or vendor task id
        task_ref: String,
        #[arg(long = "output-dir")]
        output_dir: Option<String>,
        #[arg(long = "save")]
        save: bool,
        #[arg(long = "retries")]
        retries: Option<u32>,
        #[arg(long = "output-format", default_value = "json")]
        output_format: String,
    },
}

pub fn run(cmd: TaskCmd) -> Result<()> {
    match cmd.action {
        TaskAction::List { limit, output_format } => run_list(limit, output_format),
        TaskAction::Show { task_ref, output_format } => run_show(&task_ref, output_format),
        TaskAction::Wait { task_ref, output_dir, save, retries, output_format } => {
            run_wait(task_ref, output_dir, save, retries, output_format)
        }
    }
}

/// Resolve local id or vendor task id for CLI commands.
pub fn resolve_task_ref(reference: &str) -> Result<String> {
    Database::open()?.resolve_video_task_ref(reference)
}

fn run_list(limit: usize, output_format: String) -> Result<()> {
    let rows = refreshed_video_tasks(limit)?;
    match output_format.to_lowercase().as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&rows)?),
        "plain" | "table" => print_task_list_table(&rows),
        other => anyhow::bail!("unknown output format: {other}"),
    }
    Ok(())
}

fn refreshed_video_tasks(limit: usize) -> Result<Vec<VideoTaskRecord>> {
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

fn print_task_list_table(rows: &[VideoTaskRecord]) {
    if rows.is_empty() {
        println!("No video tasks recorded.");
        return;
    }
    println!("{}", render_task_list_table(rows));
}

fn render_task_list_table(rows: &[VideoTaskRecord]) -> String {
    let width = crossterm::terminal::size()
        .map(|(width, _)| width.clamp(120, 180))
        .unwrap_or(140);
    render_task_list_table_with_width(rows, width)
}

fn render_task_list_table_with_width(rows: &[VideoTaskRecord], width: u16) -> String {
    let height = rows.len().saturating_add(3).min(u16::MAX as usize) as u16;
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);

    let header = Row::new([
        Cell::from("ID"),
        Cell::from("QUERY ID"),
        Cell::from("PHASE"),
        Cell::from("STATUS"),
        Cell::from("PROGRESS"),
        Cell::from("PROMPT"),
        Cell::from("URI"),
    ])
    .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
    .height(1);

    let table_rows = rows.iter().map(task_table_row);
    let table = Table::new(
        table_rows,
        [
            Constraint::Length(5),
            Constraint::Length(28),
            Constraint::Length(11),
            Constraint::Length(12),
            Constraint::Length(9),
            Constraint::Length(24),
            Constraint::Min(20),
        ],
    )
    .block(Block::default().title("Video Tasks").borders(Borders::ALL))
    .header(header)
    .column_spacing(1);

    table.render(area, &mut buffer);
    buffer_to_string(&buffer)
}

fn task_table_row(row: &VideoTaskRecord) -> Row<'static> {
    Row::new([
        Cell::from(row.id.to_string()),
        Cell::from(truncate_display(&row.task_id, 26)),
        Cell::from(row.phase.clone()),
        Cell::from(row.status.clone()),
        Cell::from(format_progress(row.progress)),
        Cell::from(truncate_display(&row.prompt.clone().unwrap_or_default(), 22)),
        Cell::from(truncate_display(row.uri.as_deref().unwrap_or("-"), 64)),
    ])
}

fn format_progress(progress: Option<i32>) -> String {
    match progress {
        Some(progress) => format!("{}%", progress.clamp(0, 100)),
        None => "-".to_string(),
    }
}

fn buffer_to_string(buffer: &Buffer) -> String {
    let width = buffer.area.width;
    let height = buffer.area.height;
    let mut out = String::new();
    for y in 0..height {
        let mut line = String::new();
        for x in 0..width {
            line.push_str(buffer[(x, y)].symbol());
        }
        out.push_str(line.trim_end());
        if y + 1 < height {
            out.push('\n');
        }
    }
    out
}

fn truncate_display(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return "...".chars().take(max_chars).collect();
    }
    format!("{}...", chars.into_iter().take(max_chars - 3).collect::<String>())
}

fn run_show(task_ref: &str, output_format: String) -> Result<()> {
    let task_id = resolve_task_ref(task_ref)?;
    let cfg = AppConfig::load()?;
    let api = ApiClient::from_config(cfg)?;
    let record = refresh_video_task(&api, &task_id)?;
    print_task_record(&record, output_format)
}

fn run_wait(
    task_ref: String,
    output_dir: Option<String>,
    save: bool,
    retries: Option<u32>,
    output_format: String,
) -> Result<()> {
    let task_id = resolve_task_ref(&task_ref)?;
    let cfg = AppConfig::load()?;
    let format = parse_output_format(&output_format)?;
    let api = ApiClient::with_overrides(cfg, output_dir, None, retries)?;
    wait_video_task(&api, &task_id, save, format)?;
    Ok(())
}

fn print_task_record(record: &crate::db::VideoTaskRecord, output_format: String) -> Result<()> {
    match output_format.to_lowercase().as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(record)?),
        "plain" => {
            println!("id={}", record.id);
            println!("task_id={}", record.task_id);
            println!("phase={}", record.phase);
            println!("status={}", record.status);
            if let Some(p) = record.progress {
                println!("progress={p}");
            }
            if let Some(ref uri) = record.uri {
                println!("uri={uri}");
            }
            if let Some(ref asset) = record.asset_uri {
                println!("asset_uri={asset}");
            }
        }
        other => anyhow::bail!("unknown output format: {other}"),
    }
    Ok(())
}

fn parse_output_format(s: &str) -> Result<OutputFormat> {
    match s.to_lowercase().as_str() {
        "json" => Ok(OutputFormat::Json),
        "plain" => Ok(OutputFormat::Plain),
        other => anyhow::bail!("unknown output format: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_display_short() {
        assert_eq!(truncate_display("hello", 10), "hello");
    }

    #[test]
    fn truncate_display_long() {
        assert_eq!(truncate_display("abcdefghijk", 10), "abcdefg...");
    }

    #[test]
    fn progress_formats_percent_or_dash() {
        assert_eq!(format_progress(Some(42)), "42%");
        assert_eq!(format_progress(Some(120)), "100%");
        assert_eq!(format_progress(None), "-");
    }

    #[test]
    fn task_table_renders_progress_column() {
        let rows = vec![sample_task()];
        let table = render_task_list_table_with_width(&rows, 120);

        assert!(table.contains("Video Tasks"));
        assert!(table.contains("PROGRESS"));
        assert!(table.contains("42%"));
        assert!(table.contains("video_abc"));
    }

    #[test]
    fn task_list_json_includes_progress() {
        let rows = vec![sample_task()];
        let value = serde_json::to_value(&rows).unwrap();

        assert_eq!(value[0]["progress"], 42);
    }

    fn sample_task() -> VideoTaskRecord {
        VideoTaskRecord {
            id: 7,
            task_id: "video_abc".into(),
            status: "in_progress".into(),
            phase: "processing".into(),
            prompt: Some("A red sphere rolls across a table".into()),
            input_json: None,
            progress: Some(42),
            uri: Some("https://example.com/video.mp4".into()),
            asset_uri: None,
            error: None,
            created_at: "2026-06-06T00:00:00Z".into(),
            updated_at: "2026-06-06T00:01:00Z".into(),
        }
    }
}
