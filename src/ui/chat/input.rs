use std::time::{Duration, Instant};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::commands::{SLASH_COMMANDS, command_hint_for_input, skill_name_matches};

#[derive(Debug, Clone)]
pub(crate) struct InputState {
    pub buffer: String,
    pub cursor: usize,
    pub blink_on: bool,
    pub blink_at: Instant,
    pub history: Vec<String>,
    pub history_cursor: Option<usize>,
    pub history_draft: String,
    pub completion_hint: Option<String>,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            blink_on: true,
            blink_at: Instant::now(),
            history: Vec::new(),
            history_cursor: None,
            history_draft: String::new(),
            completion_hint: None,
        }
    }
}

impl InputState {
    pub fn record_history(&mut self, input: &str) {
        if input.is_empty() {
            return;
        }
        if self.history.last().is_some_and(|last| last == input) {
            return;
        }
        self.history.push(input.to_string());
    }

    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        if self.history_cursor.is_none() {
            self.history_draft = self.buffer.clone();
            self.history_cursor = Some(self.history.len() - 1);
        } else if let Some(index) = self.history_cursor
            && index > 0
        {
            self.history_cursor = Some(index - 1);
        }
        if let Some(index) = self.history_cursor {
            self.buffer = self.history[index].clone();
            self.sync_cursor_end();
            self.completion_hint = command_hint_for_input(&self.buffer);
        }
    }

    pub fn history_next(&mut self) {
        let Some(index) = self.history_cursor else {
            return;
        };
        if index + 1 >= self.history.len() {
            self.history_cursor = None;
            self.buffer = self.history_draft.clone();
        } else {
            self.history_cursor = Some(index + 1);
            self.buffer = self.history[index + 1].clone();
        }
        self.sync_cursor_end();
        self.completion_hint = command_hint_for_input(&self.buffer);
    }

    pub fn reset_history_navigation(&mut self) {
        self.history_cursor = None;
        self.history_draft.clear();
    }

    fn char_count(&self) -> usize {
        self.buffer.chars().count()
    }

    pub fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor.min(self.char_count());
    }

    pub fn cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn cursor_right(&mut self) {
        if self.cursor < self.char_count() {
            self.cursor += 1;
        }
    }

    pub fn insert(&mut self, ch: char) {
        let before: String = self.buffer.chars().take(self.cursor).collect();
        let after: String = self.buffer.chars().skip(self.cursor).collect();
        self.buffer = format!("{before}{ch}{after}");
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let before: String = self.buffer.chars().take(self.cursor.saturating_sub(1)).collect();
        let after: String = self.buffer.chars().skip(self.cursor).collect();
        self.buffer = format!("{before}{after}");
        self.cursor -= 1;
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.char_count() {
            return;
        }
        let before: String = self.buffer.chars().take(self.cursor).collect();
        let after: String = self.buffer.chars().skip(self.cursor + 1).collect();
        self.buffer = format!("{before}{after}");
    }

    pub fn sync_cursor_end(&mut self) {
        self.cursor = self.char_count();
    }

    pub fn tick_blink(&mut self) {
        if self.blink_at.elapsed() >= Duration::from_millis(530) {
            self.blink_on = !self.blink_on;
            self.blink_at = Instant::now();
        }
    }

    pub fn complete_command(&mut self) {
        if !self.buffer.starts_with('/') {
            return;
        }
        if let Some(prefix) = self.buffer.strip_prefix("/skill ").map(str::to_string) {
            let matches = skill_name_matches(prefix.trim());
            match matches.as_slice() {
                [only] => {
                    self.buffer = format!("/skill {only} ");
                    self.sync_cursor_end();
                    self.completion_hint = None;
                }
                [] => {
                    self.completion_hint = Some("No matching skill".into());
                }
                many => {
                    self.completion_hint = Some(format!("Skills: {}", many.join("  ")));
                }
            }
            return;
        }
        let prefix = self.buffer.split_whitespace().next().unwrap_or("");
        let matches: Vec<&str> = SLASH_COMMANDS
            .iter()
            .copied()
            .filter(|command| command.starts_with(prefix))
            .collect();
        match matches.as_slice() {
            [only] => {
                self.buffer = format!("{only} ");
                self.sync_cursor_end();
                self.completion_hint = None;
            }
            [] => {
                self.completion_hint = Some("No matching command".into());
            }
            many => {
                self.completion_hint = Some(format!("Completions: {}", many.join("  ")));
            }
        }
    }
}

pub(crate) fn line_display_width(text: &str) -> usize {
    Line::from(text).width()
}

pub(crate) fn input_viewport(prompt: &str, input: &str, cursor: usize, max_width: usize) -> (String, usize) {
    let chars: Vec<char> = input.chars().collect();
    let cursor = cursor.min(chars.len());

    for skip in 0..=cursor.min(chars.len()) {
        let visible: String = chars[skip..].iter().collect();
        let display = format!("{prompt}{visible}");
        if line_display_width(&display) > max_width {
            continue;
        }
        let before: String = chars[skip..cursor].iter().collect();
        let col = line_display_width(&format!("{prompt}{before}"));
        if col <= max_width {
            return (display, col);
        }
    }

    for skip in 0..chars.len() {
        let visible: String = chars[skip..].iter().collect();
        let display = format!("{prompt}{visible}");
        if line_display_width(&display) <= max_width {
            let before: String = chars[skip..cursor.min(chars.len())].iter().collect();
            let col = line_display_width(&format!("{prompt}{before}"));
            return (display, col.min(max_width));
        }
    }

    (prompt.to_string(), prompt.len().min(max_width))
}

pub(crate) fn build_input_line(display: &str, cursor_col: usize, blink_on: bool) -> Line<'static> {
    let (before, after) = split_line_at_display_col(display, cursor_col);
    let cursor_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::REVERSED);
    if blink_on {
        let cursor_cell = after
            .chars()
            .next()
            .map(|ch| ch.to_string())
            .unwrap_or_else(|| " ".to_string());
        let rest: String = after.chars().skip(1).collect();
        Line::from(vec![
            Span::raw(before),
            Span::styled(cursor_cell, cursor_style),
            Span::raw(rest),
        ])
    } else {
        Line::from(vec![
            Span::raw(before),
            Span::styled("▏", Style::default().fg(Color::Cyan)),
            Span::raw(after),
        ])
    }
}

pub(crate) fn split_line_at_display_col(text: &str, target_col: usize) -> (String, String) {
    if target_col == 0 {
        return (String::new(), text.to_string());
    }
    let mut col = 0usize;
    let mut split_at = 0usize;
    for (idx, ch) in text.char_indices() {
        let ch_width = line_display_width(&ch.to_string());
        if col + ch_width > target_col {
            break;
        }
        col += ch_width;
        split_at = idx + ch.len_utf8();
        if col >= target_col {
            break;
        }
    }
    let before = text[..split_at].to_string();
    let after = text[split_at..].to_string();
    (before, after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_cursor_accounts_for_wide_characters() {
        let (_, col) = input_viewport("> ", "帮我打开", 4, 80);
        assert_eq!(col, 10);
    }

    #[test]
    fn input_cursor_moves_with_left_and_right() {
        let mut input = InputState { buffer: "abcd".into(), cursor: 4, ..InputState::default() };
        input.cursor_left();
        assert_eq!(input.cursor, 3);
        input.insert('X');
        assert_eq!(input.buffer, "abcXd");
        assert_eq!(input.cursor, 4);
    }

    #[test]
    fn input_line_scrolls_to_show_tail_when_overflowing() {
        let long = "a".repeat(200);
        let display = input_viewport("> ", &long, long.chars().count(), 20).0;
        assert!(line_display_width(&display) <= 20);
        assert!(display.ends_with('a'));
    }

    #[test]
    fn history_recalls_previous_entries() {
        let mut input = InputState::default();
        input.record_history("first prompt");
        input.record_history("second prompt");

        input.history_prev();
        assert_eq!(input.buffer, "second prompt");

        input.history_prev();
        assert_eq!(input.buffer, "first prompt");

        input.history_next();
        assert_eq!(input.buffer, "second prompt");

        input.history_next();
        assert_eq!(input.buffer, "");
    }

    #[test]
    fn history_restores_draft_after_browsing() {
        let mut input = InputState { buffer: "draft".into(), ..InputState::default() };
        input.record_history("earlier");

        input.history_prev();
        assert_eq!(input.buffer, "earlier");

        input.history_next();
        assert_eq!(input.buffer, "draft");
    }

    #[test]
    fn history_skips_duplicate_submissions() {
        let mut input = InputState::default();
        input.record_history("same");
        input.record_history("same");

        assert_eq!(input.history.len(), 1);
    }
}
