use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::media::input::ImageInput;
use crate::media::input::classify_input;

pub const IMAGE_FORM_HELP: &str = "F5 submit  Tab cycles Prompt/Params/Refs/Output";
pub const VIDEO_FORM_HELP: &str = "F5 submit  Tab cycles sections  t/r/g on Params/Refs";

use crossterm::event::{KeyCode, KeyModifiers};

/// Submit shortcuts. F5 / Ctrl+S work in WSL terminals where Ctrl+Enter often does not.
pub fn is_submit_key(key: KeyCode, modifiers: KeyModifiers) -> bool {
    if key == KeyCode::F(5) {
        return true;
    }
    if modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key, KeyCode::Char('s') | KeyCode::Char('j') | KeyCode::Enter)
    {
        return true;
    }
    false
}

#[derive(Debug, Clone)]
pub struct TextInput {
    pub value: String,
    pub cursor: usize,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.chars().count();
        Self { value, cursor }
    }

    pub fn empty() -> Self {
        Self { value: String::new(), cursor: 0 }
    }

    pub fn push_char(&mut self, c: char) {
        let byte_idx = char_index_to_byte(&self.value, self.cursor);
        self.value.insert(byte_idx, c);
        self.cursor += 1;
    }

    pub fn pop_char(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let byte_idx = char_index_to_byte(&self.value, self.cursor);
        self.value.remove(byte_idx);
    }

    #[allow(dead_code)]
    pub fn move_cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    #[allow(dead_code)]
    pub fn move_cursor_right(&mut self) {
        if self.cursor < self.value.chars().count() {
            self.cursor += 1;
        }
    }

    #[allow(dead_code)]
    pub fn move_cursor_home(&mut self) {
        self.cursor = 0;
    }

    #[allow(dead_code)]
    pub fn move_cursor_end(&mut self) {
        self.cursor = self.value.chars().count();
    }

    pub fn display_with_cursor(&self, editing: bool) -> String {
        if !editing {
            return if self.value.is_empty() {
                "(empty)".into()
            } else {
                self.value.clone()
            };
        }
        let chars: Vec<char> = self.value.chars().collect();
        let mut out = String::new();
        for (i, ch) in chars.iter().enumerate() {
            if i == self.cursor {
                out.push('|');
            }
            out.push(*ch);
        }
        if self.cursor >= chars.len() {
            out.push('|');
        }
        if out.is_empty() {
            out.push('|');
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct TextArea {
    pub lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

impl TextArea {
    pub fn new() -> Self {
        Self { lines: vec![String::new()], cursor_row: 0, cursor_col: 0 }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn push_char(&mut self, c: char) {
        if c == '\n' {
            self.split_line();
            return;
        }
        let line = &mut self.lines[self.cursor_row];
        let byte_idx = char_index_to_byte(line, self.cursor_col);
        line.insert(byte_idx, c);
        self.cursor_col += 1;
    }

    fn split_line(&mut self) {
        let line = self.lines[self.cursor_row].clone();
        let byte_idx = char_index_to_byte(&line, self.cursor_col);
        let tail = line[byte_idx..].to_string();
        self.lines[self.cursor_row] = line[..byte_idx].to_string();
        self.cursor_row += 1;
        self.lines.insert(self.cursor_row, tail);
        self.cursor_col = 0;
    }

    pub fn pop_char(&mut self) {
        if self.cursor_col > 0 {
            let line = &mut self.lines[self.cursor_row];
            self.cursor_col -= 1;
            let byte_idx = char_index_to_byte(line, self.cursor_col);
            line.remove(byte_idx);
            return;
        }
        if self.cursor_row == 0 {
            return;
        }
        let current = self.lines.remove(self.cursor_row);
        self.cursor_row -= 1;
        self.cursor_col = self.lines[self.cursor_row].chars().count();
        self.lines[self.cursor_row].push_str(&current);
    }

    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].chars().count();
        }
    }

    pub fn move_right(&mut self) {
        let len = self.lines[self.cursor_row].chars().count();
        if self.cursor_col < len {
            self.cursor_col += 1;
        } else if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
    }

    /// Render lines for the TUI; when `editing`, inserts a `|` at the cursor.
    pub fn display_lines(&self, editing: bool) -> Vec<Line<'static>> {
        if !editing {
            if self.text().trim().is_empty() {
                return vec![Line::from(Span::styled(
                    "(empty — type to edit)",
                    Style::default().fg(Color::DarkGray),
                ))];
            }
            return self.lines.iter().map(|line| Line::from(line.clone())).collect();
        }

        let cursor_style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
        self.lines
            .iter()
            .enumerate()
            .map(|(row, line)| {
                let chars: Vec<char> = line.chars().collect();
                let mut spans = Vec::new();
                for (col, ch) in chars.iter().enumerate() {
                    if row == self.cursor_row && col == self.cursor_col {
                        spans.push(Span::styled("|", cursor_style));
                    }
                    spans.push(Span::raw(ch.to_string()));
                }
                if row == self.cursor_row && self.cursor_col >= chars.len() {
                    spans.push(Span::styled("|", cursor_style));
                }
                if spans.is_empty() {
                    spans.push(Span::styled("|", cursor_style));
                }
                Line::from(spans)
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct SelectField {
    pub options: Vec<String>,
    pub index: usize,
    pub disabled: bool,
}

impl SelectField {
    pub fn new(options: Vec<String>, default_index: usize) -> Self {
        let index = if options.is_empty() {
            0
        } else {
            default_index.min(options.len() - 1)
        };
        Self { options, index, disabled: false }
    }

    pub fn current(&self) -> &str {
        self.options.get(self.index).map(String::as_str).unwrap_or("-")
    }

    pub fn next(&mut self) {
        if self.disabled || self.options.is_empty() {
            return;
        }
        self.index = (self.index + 1) % self.options.len();
    }

    pub fn previous(&mut self) {
        if self.disabled || self.options.is_empty() {
            return;
        }
        self.index = if self.index == 0 {
            self.options.len() - 1
        } else {
            self.index - 1
        };
    }
}

#[derive(Debug, Clone)]
pub struct InputListItem {
    pub raw: String,
    pub valid: bool,
    pub kind_label: String,
}

impl InputListItem {
    pub fn from_raw(raw: String, video_only_url: bool) -> Self {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Self { raw, valid: false, kind_label: "empty".into() };
        }
        if trimmed.starts_with("asset://") {
            return Self { raw: trimmed.to_string(), valid: true, kind_label: "asset".into() };
        }
        let classified = classify_input(trimmed);
        let (valid, kind_label) = match classified {
            ImageInput::Url(_) if video_only_url => (true, "url".into()),
            ImageInput::Url(_) => (true, "url".into()),
            ImageInput::LocalPath(_) if video_only_url => (false, "local".into()),
            ImageInput::LocalPath(_) => (true, "local".into()),
            ImageInput::Base64(_) if video_only_url => (false, "b64".into()),
            ImageInput::Base64(_) => (true, "b64".into()),
            ImageInput::DataUri(_) if video_only_url => (false, "data".into()),
            ImageInput::DataUri(_) => (true, "data".into()),
        };
        Self { raw: trimmed.to_string(), valid, kind_label }
    }
}

#[derive(Debug, Clone)]
pub struct InputList {
    pub items: Vec<InputListItem>,
    pub selected: usize,
    pub add_buffer: TextInput,
    pub adding: bool,
}

impl InputList {
    pub fn new() -> Self {
        Self { items: Vec::new(), selected: 0, add_buffer: TextInput::empty(), adding: false }
    }

    pub fn raw_values(&self) -> Vec<String> {
        self.items.iter().map(|i| i.raw.clone()).collect()
    }

    pub fn push_raw(&mut self, raw: String, video_only_url: bool) {
        self.items.push(InputListItem::from_raw(raw, video_only_url));
        self.selected = self.items.len().saturating_sub(1);
    }

    pub fn remove_selected(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.items.remove(self.selected);
        if self.selected >= self.items.len() && !self.items.is_empty() {
            self.selected = self.items.len() - 1;
        }
    }
}

pub fn form_field_line(label: &str, focused: bool, editing: bool, value: &str) -> Line<'static> {
    let prefix = if focused { "▸ " } else { "  " };
    let label_style = if focused {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let value_style = if editing {
        Style::default().fg(Color::Yellow)
    } else if focused {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::Gray)
    };
    Line::from(vec![
        Span::styled(format!("{prefix}{label:<16}"), label_style),
        Span::styled(value.to_string(), value_style),
    ])
}

pub fn move_field_focus(current: usize, len: usize, delta: i32) -> usize {
    if len == 0 {
        return 0;
    }
    let next = (current as i32 + delta).rem_euclid(len as i32);
    next as usize
}

fn char_index_to_byte(s: &str, char_index: usize) -> usize {
    s.char_indices().nth(char_index).map(|(i, _)| i).unwrap_or(s.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_input_cursor_insert_delete() {
        let mut input = TextInput::empty();
        input.push_char('a');
        input.push_char('b');
        assert_eq!(input.value, "ab");
        input.pop_char();
        assert_eq!(input.value, "a");
    }

    #[test]
    fn select_cycles_options() {
        let mut select = SelectField::new(vec!["1:1".into(), "16:9".into()], 0);
        select.next();
        assert_eq!(select.current(), "16:9");
        select.previous();
        assert_eq!(select.current(), "1:1");
    }

    #[test]
    fn video_input_rejects_local_path() {
        let item = InputListItem::from_raw("/tmp/a.png".into(), true);
        assert!(!item.valid);
        let item = InputListItem::from_raw("https://example.com/a.png".into(), true);
        assert!(item.valid);
    }

    #[test]
    fn move_field_focus_wraps() {
        assert_eq!(move_field_focus(0, 3, -1), 2);
        assert_eq!(move_field_focus(2, 3, 1), 0);
    }

    #[test]
    fn is_submit_key_f5() {
        assert!(is_submit_key(KeyCode::F(5), KeyModifiers::NONE));
    }

    #[test]
    fn is_submit_key_ctrl_s() {
        assert!(is_submit_key(KeyCode::Char('s'), KeyModifiers::CONTROL));
    }

    #[test]
    fn textarea_shows_cursor_when_editing() {
        let mut area = TextArea::new();
        area.push_char('a');
        let lines = area.display_lines(true);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].to_string().contains("|"));
    }
}
