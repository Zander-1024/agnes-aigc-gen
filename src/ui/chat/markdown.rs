use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::style_bridge::convert_core_style;
use super::wrap::wrap_styled_line;

pub(crate) fn render_assistant_markdown(text: &str, width: usize) -> Vec<Line<'static>> {
    let md = tui_markdown::from_str(text);
    let mut out = Vec::new();
    for (idx, line) in md.lines.into_iter().enumerate() {
        let converted = Line::from(
            line.spans
                .into_iter()
                .map(|span| Span::styled(span.content.into_owned(), convert_core_style(span.style)))
                .collect::<Vec<_>>(),
        );
        let mut wrapped = wrap_styled_line(converted, width.saturating_sub(2));
        if idx == 0 && !wrapped.is_empty() {
            let first = wrapped.remove(0);
            let mut spans = vec![Span::styled("● ", Style::default().fg(Color::Green))];
            spans.extend(first.spans);
            wrapped.insert(0, Line::from(spans));
        }
        out.extend(wrapped);
    }
    if out.is_empty() {
        out.push(Line::from(Span::styled("● ", Style::default().fg(Color::Green))));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    #[test]
    fn assistant_markdown_renders_bold() {
        let lines = render_assistant_markdown("**bold**", 80);
        assert!(lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|span| span.content.contains("bold") && span.style.add_modifier.contains(Modifier::BOLD))
        }));
    }
}
