use crossterm::clipboard::CopyToClipboard;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, TableState};
use std::io;
use std::time::Duration;

use anyhow::Result;

pub const DETAIL_DISPLAY_MAX_CHARS: usize = 200;
pub const LIST_HELP_TEXT: &str = "Up/Down row  Left/Right field  Enter copy full  Home/End jump  q/Esc quit";
pub const LIST_PAGE_ROWS: usize = 10;

/// Shared row selection + detail-field index for list TUI screens.
pub struct ListTable<T> {
    pub rows: Vec<T>,
    table_state: TableState,
    pub detail_field: usize,
}

impl<T> ListTable<T> {
    pub fn new(rows: Vec<T>) -> Self {
        Self::with_detail_field(rows, 0)
    }

    pub fn with_detail_field(rows: Vec<T>, detail_field: usize) -> Self {
        let mut table_state = TableState::default();
        if !rows.is_empty() {
            table_state.select(Some(0));
        }
        Self { rows, table_state, detail_field }
    }

    pub fn table_state_mut(&mut self) -> &mut TableState {
        &mut self.table_state
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.table_state.selected().filter(|index| *index < self.rows.len())
    }

    pub fn selected_row(&self) -> Option<&T> {
        self.selected_index().and_then(|index| self.rows.get(index))
    }

    pub fn selected_detail_copy_value<F>(&self, value_fn: F) -> Option<&str>
    where
        F: Fn(&T, usize) -> Option<&str>,
    {
        let row = self.selected_row()?;
        value_fn(row, self.detail_field)
    }

    pub fn selected_detail_display<F>(&self, value_fn: F) -> String
    where
        F: Fn(&T, usize) -> Option<&str>,
    {
        detail_value_display(self.selected_detail_copy_value(value_fn).unwrap_or("-"))
    }

    pub fn select_next(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        let next = self
            .selected_index()
            .map_or(0, |index| index.saturating_add(1).min(self.rows.len() - 1));
        self.table_state.select(Some(next));
    }

    pub fn select_previous(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        let previous = self.selected_index().map_or(0, |index| index.saturating_sub(1));
        self.table_state.select(Some(previous));
    }

    pub fn select_first(&mut self) {
        if !self.rows.is_empty() {
            self.table_state.select(Some(0));
        }
    }

    pub fn select_last(&mut self) {
        if !self.rows.is_empty() {
            self.table_state.select(Some(self.rows.len() - 1));
        }
    }

    pub fn select_next_detail(&mut self, field_count: usize) {
        if field_count == 0 {
            return;
        }
        self.detail_field = (self.detail_field + 1) % field_count;
    }

    pub fn select_previous_detail(&mut self, field_count: usize) {
        if field_count == 0 {
            return;
        }
        self.detail_field = if self.detail_field == 0 {
            field_count - 1
        } else {
            self.detail_field - 1
        };
    }

    pub fn apply_key_action(&mut self, action: &ListKeyAction, field_count: usize) {
        if action.row_up {
            self.select_previous();
        }
        if action.row_down {
            self.select_next();
        }
        if action.row_first {
            self.select_first();
        }
        if action.row_last {
            self.select_last();
        }
        if action.row_page_up {
            for _ in 0..LIST_PAGE_ROWS {
                self.select_previous();
            }
        }
        if action.row_page_down {
            for _ in 0..LIST_PAGE_ROWS {
                self.select_next();
            }
        }
        if action.field_previous {
            self.select_previous_detail(field_count);
        }
        if action.field_next {
            self.select_next_detail(field_count);
        }
    }
}

pub struct ListLayoutChunks {
    pub table: Rect,
    pub detail: Rect,
    pub help: Rect,
}

pub fn split_list_layout(area: Rect) -> ListLayoutChunks {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(4), Constraint::Length(1)])
        .split(area);
    ListLayoutChunks { table: chunks[0], detail: chunks[1], help: chunks[2] }
}

pub fn render_list_detail_help(
    frame: &mut Frame,
    detail_area: Rect,
    help_area: Rect,
    detail_labels: &[&str],
    detail_field_index: usize,
    detail_display: String,
    help_text: &str,
) {
    let tabs = detail_field_tabs_line(detail_labels, detail_field_index);
    render_detail_panel(frame, detail_area, tabs, detail_display);
    render_help_line(frame, help_area, help_text);
}

pub fn poll_list_key_event() -> Result<Option<(KeyCode, KeyModifiers)>> {
    if event::poll(Duration::from_millis(120))?
        && let Event::Key(key) = event::read()?
        && key.kind == KeyEventKind::Press
    {
        return Ok(Some((key.code, key.modifiers)));
    }
    Ok(None)
}

pub fn apply_list_key<T, F>(
    key: KeyCode,
    modifiers: KeyModifiers,
    state: &mut ListTable<T>,
    field_count: usize,
    detail_value: F,
) -> bool
where
    F: Fn(&T, usize) -> Option<&str>,
{
    let action = parse_list_key(key, modifiers);
    if action.quit {
        return true;
    }
    state.apply_key_action(&action, field_count);
    if action.copy
        && let Some(value) = state.selected_detail_copy_value(detail_value)
    {
        copy_to_clipboard_silent(value);
    }
    false
}

pub fn run_list_event_loop<T, DrawFn, ValueFn>(
    terminal: &mut DefaultTerminal,
    mut state: ListTable<T>,
    field_count: usize,
    detail_value: ValueFn,
    mut draw: DrawFn,
) -> Result<()>
where
    DrawFn: FnMut(&mut Frame, &mut ListTable<T>),
    ValueFn: Fn(&T, usize) -> Option<&str>,
{
    loop {
        terminal.draw(|frame| draw(frame, &mut state))?;
        if let Some((code, modifiers)) = poll_list_key_event()?
            && apply_list_key(code, modifiers, &mut state, field_count, &detail_value)
        {
            break;
        }
    }
    Ok(())
}

pub fn run_list_tui<T, DrawFn, ValueFn>(
    rows: Vec<T>,
    empty_message: &str,
    field_count: usize,
    detail_value: ValueFn,
    draw: DrawFn,
) -> Result<()>
where
    DrawFn: FnMut(&mut Frame, &mut ListTable<T>),
    ValueFn: Fn(&T, usize) -> Option<&str>,
{
    if rows.is_empty() {
        println!("{empty_message}");
        return Ok(());
    }
    let mut terminal = ratatui::try_init()?;
    let result = run_list_event_loop(&mut terminal, ListTable::new(rows), field_count, detail_value, draw);
    ratatui::restore();
    result
}

pub fn truncate_display(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return "...".chars().take(max_chars).collect();
    }
    format!("{}...", chars.into_iter().take(max_chars - 3).collect::<String>())
}

pub fn detail_value_display(full: &str) -> String {
    truncate_display(full, DETAIL_DISPLAY_MAX_CHARS)
}

pub fn detail_field_tabs_line(labels: &[&str], selected_index: usize) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, label) in labels.iter().enumerate() {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        let style = if index == selected_index {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(format!(" {label} "), style));
    }
    Line::from(spans)
}

pub fn render_detail_panel(frame: &mut Frame, area: Rect, tabs_line: Line<'_>, value_text: String) {
    let block = Block::default().title("Detail").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    let tabs = Paragraph::new(tabs_line);
    frame.render_widget(tabs, chunks[0]);

    let value = Paragraph::new(value_text).style(Style::default().fg(Color::White));
    frame.render_widget(value, chunks[1]);
}

pub fn render_help_line(frame: &mut Frame, area: Rect, text: &str) {
    let help = Paragraph::new(text);
    frame.render_widget(help, area);
}

pub fn copy_to_clipboard_silent(text: &str) {
    if text.trim().is_empty() {
        return;
    }
    let _ = execute!(io::stdout(), CopyToClipboard::to_clipboard_from(text));
}

pub struct ListKeyAction {
    pub quit: bool,
    pub row_up: bool,
    pub row_down: bool,
    pub row_first: bool,
    pub row_last: bool,
    pub row_page_up: bool,
    pub row_page_down: bool,
    pub field_previous: bool,
    pub field_next: bool,
    pub copy: bool,
}

impl ListKeyAction {
    pub fn none() -> Self {
        Self {
            quit: false,
            row_up: false,
            row_down: false,
            row_first: false,
            row_last: false,
            row_page_up: false,
            row_page_down: false,
            field_previous: false,
            field_next: false,
            copy: false,
        }
    }
}

pub fn parse_list_key(key: KeyCode, modifiers: KeyModifiers) -> ListKeyAction {
    let mut action = ListKeyAction::none();
    match key {
        KeyCode::Esc | KeyCode::Char('q') => action.quit = true,
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => action.quit = true,
        KeyCode::Up => action.row_up = true,
        KeyCode::Down => action.row_down = true,
        KeyCode::Home => action.row_first = true,
        KeyCode::End => action.row_last = true,
        KeyCode::PageUp => action.row_page_up = true,
        KeyCode::PageDown => action.row_page_down = true,
        KeyCode::Left => action.field_previous = true,
        KeyCode::Right => action.field_next = true,
        KeyCode::Enter => action.copy = true,
        _ => {}
    }
    action
}

pub fn buffer_to_string(buffer: &ratatui::buffer::Buffer) -> String {
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
    fn detail_value_display_caps_at_200_chars() {
        let long = "a".repeat(250);
        let displayed = detail_value_display(&long);
        assert_eq!(displayed.chars().count(), 200);
        assert!(displayed.ends_with("..."));
    }

    #[test]
    fn detail_tabs_line_has_no_value_suffix() {
        let line = detail_field_tabs_line(&["QUERY ID", "PROMPT", "URI"], 1);
        let text = line.to_string();
        assert!(text.contains("PROMPT"));
        assert!(!text.contains('|'));
        assert!(!text.contains("PROMPT:"));
    }

    #[test]
    fn list_table_tracks_selected_row() {
        #[derive(Clone)]
        struct Row {
            id: i64,
        }

        let mut state = ListTable::new(vec![Row { id: 1 }, Row { id: 2 }]);

        assert_eq!(state.selected_row().unwrap().id, 1);
        state.select_next();
        assert_eq!(state.selected_row().unwrap().id, 2);
        state.select_previous();
        assert_eq!(state.selected_row().unwrap().id, 1);
        state.select_last();
        assert_eq!(state.selected_row().unwrap().id, 2);
        state.select_first();
        assert_eq!(state.selected_row().unwrap().id, 1);
    }

    #[test]
    fn list_table_detail_field_cycles() {
        #[derive(Clone)]
        struct Row {
            a: &'static str,
            b: &'static str,
        }

        let value = |row: &Row, field: usize| match field {
            0 => Some(row.a),
            1 => Some(row.b),
            _ => None,
        };

        let mut state = ListTable::new(vec![Row { a: "alpha", b: "beta" }]);
        assert_eq!(
            state.selected_detail_copy_value(|row, field| value(row, field)),
            Some("alpha")
        );

        state.select_next_detail(2);
        assert_eq!(state.detail_field, 1);
        assert_eq!(
            state.selected_detail_copy_value(|row, field| value(row, field)),
            Some("beta")
        );

        state.select_previous_detail(2);
        assert_eq!(state.detail_field, 0);
    }
}
