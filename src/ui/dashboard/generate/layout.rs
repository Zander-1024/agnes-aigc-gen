use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::fields::{InputList, TextArea, form_field_line};
use crate::ui::dashboard::task_display::TaskStripData;

pub fn render_text_box(frame: &mut Frame, area: Rect, title: &str, focused: bool, editing: bool, text: &TextArea) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let box_title = if editing {
        format!("{title} (F5 submit, Esc exit)")
    } else {
        title.to_string()
    };
    let lines = text.display_lines(editing);
    let block = Block::default()
        .title(box_title)
        .borders(Borders::ALL)
        .border_style(border_style);
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

pub fn render_params_panel(frame: &mut Frame, area: Rect, focused: bool, title: &str, lines: Vec<Line<'static>>) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

pub fn render_media_panel(
    frame: &mut Frame,
    area: Rect,
    focused: bool,
    inputs: &InputList,
    adding: bool,
    footer_hint: &str,
) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        footer_hint,
        Style::default().fg(Color::DarkGray),
    )));
    if adding {
        lines.push(Line::from(format!("+ {}", inputs.add_buffer.display_with_cursor(true))));
    } else if inputs.items.is_empty() {
        lines.push(Line::from("(none)"));
    } else {
        for (i, item) in inputs.items.iter().enumerate() {
            let mark = if i == inputs.selected { ">" } else { " " };
            let valid = if item.valid { "✓" } else { "✗" };
            lines.push(Line::from(format!("{mark} [{valid}] {} {}", item.kind_label, item.raw)));
        }
    }
    let block = Block::default()
        .title(format!("References ({})", inputs.items.len()))
        .borders(Borders::ALL)
        .border_style(border_style);
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

pub fn render_task_strip(frame: &mut Frame, area: Rect, data: &TaskStripData) {
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "Active (executing)",
        Style::default().fg(Color::Yellow),
    )));
    if data.active.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (none — submit async to start)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for line in &data.active {
            lines.push(line.clone());
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Recent", Style::default().fg(Color::DarkGray))));
    if data.recent.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (none)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for line in &data.recent {
            lines.push(line.clone());
        }
    }
    let block = Block::default()
        .title("Tasks (t open  r refresh  g running)")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: true }), area);
}

pub fn param_line(label: &str, field_focus: bool, section_focus: bool, value: &str) -> Line<'static> {
    form_field_line(label, section_focus && field_focus, false, value)
}

pub fn error_line(msg: &str) -> Line<'static> {
    Line::from(Span::styled(format!("Error: {msg}"), Style::default().fg(Color::Red)))
}
