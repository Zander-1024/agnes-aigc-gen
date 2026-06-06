use crossterm::clipboard::CopyToClipboard;
use crossterm::event::{KeyCode, KeyModifiers};
use crossterm::execute;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use std::io;

pub const DETAIL_DISPLAY_MAX_CHARS: usize = 200;
pub const LIST_HELP_TEXT: &str = "Up/Down row  Left/Right field  Enter copy full  Home/End jump  q/Esc quit";

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
}
