use anyhow::Result;
use base64::Engine;
use clap::{Args, Subcommand};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Widget};
use std::io::{self, IsTerminal, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

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
        "plain" => {
            if io::stdout().is_terminal() {
                run_task_list_tui(rows)?;
            } else {
                print_task_list_table(&rows);
            }
        }
        "table" => print_task_list_table(&rows),
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

struct TaskListUiState {
    rows: Vec<VideoTaskRecord>,
    table_state: TableState,
    detail_field: TaskDetailField,
}

impl TaskListUiState {
    fn new(rows: Vec<VideoTaskRecord>) -> Self {
        let mut table_state = TableState::default();
        if !rows.is_empty() {
            table_state.select(Some(0));
        }
        Self { rows, table_state, detail_field: TaskDetailField::QueryId }
    }

    fn selected_index(&self) -> Option<usize> {
        self.table_state.selected().filter(|index| *index < self.rows.len())
    }

    fn selected_row(&self) -> Option<&VideoTaskRecord> {
        self.selected_index().and_then(|index| self.rows.get(index))
    }

    fn selected_detail_value(&self) -> Option<&str> {
        let row = self.selected_row()?;
        match self.detail_field {
            TaskDetailField::QueryId => non_empty_value(Some(row.task_id.as_str())),
            TaskDetailField::Prompt => non_empty_value(row.prompt.as_deref()),
            TaskDetailField::Uri => non_empty_value(row.uri.as_deref()),
        }
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
enum TaskDetailField {
    QueryId,
    Prompt,
    Uri,
}

impl TaskDetailField {
    const ALL: [Self; 3] = [Self::QueryId, Self::Prompt, Self::Uri];

    fn label(self) -> &'static str {
        match self {
            Self::QueryId => "QUERY ID",
            Self::Prompt => "PROMPT",
            Self::Uri => "URI",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::QueryId => Self::Prompt,
            Self::Prompt => Self::Uri,
            Self::Uri => Self::QueryId,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::QueryId => Self::Uri,
            Self::Prompt => Self::QueryId,
            Self::Uri => Self::Prompt,
        }
    }
}

fn run_task_list_tui(rows: Vec<VideoTaskRecord>) -> Result<()> {
    if rows.is_empty() {
        println!("No video tasks recorded.");
        return Ok(());
    }

    let mut terminal = ratatui::try_init()?;
    let result = run_task_list_app(&mut terminal, rows);
    ratatui::restore();
    result
}

fn run_task_list_app(terminal: &mut DefaultTerminal, rows: Vec<VideoTaskRecord>) -> Result<()> {
    let mut state = TaskListUiState::new(rows);
    loop {
        terminal.draw(|frame| render_task_list_ui(frame, &mut state))?;
        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if handle_task_list_key(key.code, key.modifiers, &mut state) {
                break;
            }
        }
    }
    Ok(())
}

fn handle_task_list_key(key: KeyCode, modifiers: KeyModifiers, state: &mut TaskListUiState) -> bool {
    match key {
        KeyCode::Esc | KeyCode::Char('q') => true,
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => true,
        KeyCode::Up => {
            state.select_previous();
            false
        }
        KeyCode::Down => {
            state.select_next();
            false
        }
        KeyCode::Home => {
            state.select_first();
            false
        }
        KeyCode::End => {
            state.select_last();
            false
        }
        KeyCode::PageUp => {
            for _ in 0..10 {
                state.select_previous();
            }
            false
        }
        KeyCode::PageDown => {
            for _ in 0..10 {
                state.select_next();
            }
            false
        }
        KeyCode::Left => {
            state.select_previous_detail();
            false
        }
        KeyCode::Right => {
            state.select_next_detail();
            false
        }
        KeyCode::Enter => {
            if let Some(value) = state.selected_detail_value() {
                copy_to_clipboard_silent(value);
            }
            false
        }
        _ => false,
    }
}

fn render_task_list_ui(frame: &mut ratatui::Frame, state: &mut TaskListUiState) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let table = task_table(&state.rows)
        .block(Block::default().title("Video Tasks").borders(Borders::ALL))
        .highlight_symbol(">> ")
        .row_highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_stateful_widget(table, chunks[0], &mut state.table_state);

    let detail =
        Paragraph::new(selected_task_detail_line(state)).style(Style::default().fg(Color::White).bg(Color::DarkGray));
    frame.render_widget(detail, chunks[1]);

    let help = Paragraph::new("Up/Down row  Left/Right detail  Enter copy  Home/End jump  q/Esc quit");
    frame.render_widget(help, chunks[2]);
}

fn selected_task_detail_line(state: &TaskListUiState) -> Line<'static> {
    let mut spans = Vec::new();
    for field in TaskDetailField::ALL {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        let style = if field == state.detail_field {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(format!(" {} ", field.label()), style));
    }

    spans.push(Span::raw("  |  "));
    spans.push(Span::styled(
        format!("{}: ", state.detail_field.label()),
        Style::default().add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(state.selected_detail_value().unwrap_or("-").to_string()));
    Line::from(spans)
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

    let table = task_table(rows).block(Block::default().title("Video Tasks").borders(Borders::ALL));

    table.render(area, &mut buffer);
    buffer_to_string(&buffer)
}

fn task_table(rows: &[VideoTaskRecord]) -> Table<'static> {
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

    Table::new(
        rows.iter().map(task_table_row),
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
    .header(header)
    .column_spacing(1)
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

fn non_empty_value(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() { None } else { Some(trimmed) }
    })
}

fn copy_to_clipboard_silent(text: &str) {
    if text.trim().is_empty() {
        return;
    }

    if copy_with_system_clipboard(text) {
        return;
    }
    copy_with_osc52(text);
}

fn copy_with_system_clipboard(text: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        return copy_with_command("pbcopy", &[], text);
    }

    #[cfg(target_os = "linux")]
    {
        if copy_with_command("wl-copy", &[], text) {
            return true;
        }
        if copy_with_command("xclip", &["-selection", "clipboard"], text) {
            return true;
        }
        if copy_with_command("xsel", &["--clipboard", "--input"], text) {
            return true;
        }
        return false;
    }

    #[cfg(target_os = "windows")]
    {
        return copy_with_command("clip", &[], text);
    }

    #[allow(unreachable_code)]
    false
}

fn copy_with_command(program: &str, args: &[&str], text: &str) -> bool {
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };

    let Some(mut stdin) = child.stdin.take() else {
        return false;
    };
    stdin.write_all(text.as_bytes()).is_ok()
}

fn copy_with_osc52(text: &str) {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let mut stdout = io::stdout();
    let _ = write!(stdout, "\x1b]52;c;{encoded}\x07");
    let _ = stdout.flush();
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

    #[test]
    fn task_list_state_tracks_selected_row() {
        let mut second = sample_task();
        second.id = 8;
        second.task_id = "video_def".into();
        second.uri = None;

        let mut state = TaskListUiState::new(vec![sample_task(), second]);

        assert_eq!(state.selected_row().unwrap().id, 7);
        assert_eq!(state.selected_detail_value(), Some("video_abc"));

        state.select_next();
        assert_eq!(state.selected_row().unwrap().id, 8);
        assert_eq!(state.selected_detail_value(), Some("video_def"));

        state.select_previous();
        assert_eq!(state.selected_row().unwrap().id, 7);
    }

    #[test]
    fn task_list_detail_field_cycles_copyable_fields() {
        let mut state = TaskListUiState::new(vec![sample_task()]);

        assert_eq!(state.detail_field, TaskDetailField::QueryId);
        assert_eq!(state.selected_detail_value(), Some("video_abc"));

        state.select_next_detail();
        assert_eq!(state.detail_field, TaskDetailField::Prompt);
        assert_eq!(state.selected_detail_value(), Some("A red sphere rolls across a table"));

        state.select_next_detail();
        assert_eq!(state.detail_field, TaskDetailField::Uri);
        assert_eq!(state.selected_detail_value(), Some("https://example.com/video.mp4"));

        state.select_next_detail();
        assert_eq!(state.detail_field, TaskDetailField::QueryId);
    }

    #[test]
    fn task_detail_line_is_single_line() {
        let mut state = TaskListUiState::new(vec![sample_task()]);
        state.detail_field = TaskDetailField::Uri;

        let line = selected_task_detail_line(&state);
        let text = line.to_string();

        assert!(text.contains("URI: https://example.com/video.mp4"));
        assert!(!text.contains('\n'));
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
