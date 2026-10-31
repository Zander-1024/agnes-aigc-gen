use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::markdown::render_assistant_markdown;
use super::model::ChatLineKind;
use super::state::ChatUiState;
use super::wrap::{wrap_prefixed, wrap_text};

pub(crate) const USER_BUBBLE_BG: Color = Color::Rgb(30, 60, 90);
const USER_MAX_WIDTH_PERCENT: usize = 75;

pub(crate) fn build_intro_lines(state: &ChatUiState, width: usize) -> Vec<Line<'static>> {
    let style = Style::default().fg(Color::DarkGray);
    wrap_text(&intro_text(state), width)
        .into_iter()
        .map(|line| Line::from(Span::styled(line, style)))
        .collect()
}

pub(crate) fn build_transcript_lines(state: &ChatUiState, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut thinking_hidden = 0usize;
    let mut tools_hidden = 0usize;

    for line in &state.lines {
        match line.kind {
            ChatLineKind::Thinking if !state.thinking_expanded => {
                thinking_hidden += 1;
            }
            ChatLineKind::Tool if !state.tools_expanded => {
                tools_hidden += 1;
            }
            ChatLineKind::User => {
                flush_hidden_thinking(&mut out, thinking_hidden);
                thinking_hidden = 0;
                flush_hidden_tools(&mut out, tools_hidden);
                tools_hidden = 0;
                push_turn_divider(&mut out, width);
                out.extend(render_user_bubble(&line.text, width));
            }
            ChatLineKind::Assistant => {
                flush_hidden_thinking(&mut out, thinking_hidden);
                thinking_hidden = 0;
                flush_hidden_tools(&mut out, tools_hidden);
                tools_hidden = 0;
                out.extend(render_assistant_markdown(&line.text, width));
            }
            ChatLineKind::Thinking => {
                flush_hidden_tools(&mut out, tools_hidden);
                tools_hidden = 0;
                thinking_hidden = 0;
                out.extend(render_styled_prefixed(
                    "  thinking: ",
                    &line.text,
                    width,
                    Style::default().fg(Color::DarkGray),
                ));
            }
            ChatLineKind::Tool => {
                flush_hidden_thinking(&mut out, thinking_hidden);
                thinking_hidden = 0;
                tools_hidden = 0;
                out.extend(render_styled_prefixed(
                    "  tool: ",
                    &line.text,
                    width,
                    Style::default().fg(Color::Yellow),
                ));
            }
            ChatLineKind::Status => {
                out.extend(render_styled_prefixed(
                    "  ",
                    &line.text,
                    width,
                    Style::default().fg(Color::Cyan),
                ));
            }
            ChatLineKind::Error => {
                out.extend(render_styled_prefixed(
                    "! ",
                    &line.text,
                    width,
                    Style::default().fg(Color::Red),
                ));
            }
        }
    }
    flush_hidden_thinking(&mut out, thinking_hidden);
    flush_hidden_tools(&mut out, tools_hidden);
    while out
        .last()
        .is_some_and(|line| line.spans.is_empty() || line_is_blank(line))
    {
        out.pop();
    }
    out
}

fn flush_hidden_thinking(out: &mut Vec<Line<'static>>, count: usize) {
    if count == 0 {
        return;
    }
    let text = if count == 1 {
        "• Thinking hidden (Ctrl-T to expand)".to_string()
    } else {
        format!("• Thinking hidden ×{count} (Ctrl-T to expand)")
    };
    out.push(Line::from(Span::styled(text, Style::default().fg(Color::DarkGray))));
}

fn flush_hidden_tools(out: &mut Vec<Line<'static>>, count: usize) {
    if count == 0 {
        return;
    }
    let text = if count == 1 {
        "• Tool activity hidden (Ctrl-E to expand)".to_string()
    } else {
        format!("• Tool activity hidden ×{count} (Ctrl-E to expand)")
    };
    out.push(Line::from(Span::styled(text, Style::default().fg(Color::DarkGray))));
}

fn push_turn_divider(out: &mut Vec<Line<'static>>, width: usize) {
    if out.is_empty() {
        return;
    }
    out.push(dashed_divider(width));
}

fn dashed_divider(width: usize) -> Line<'static> {
    let width = width.max(8);
    let unit = "─ ";
    let mut line = String::new();
    while line.chars().count() < width {
        line.push_str(unit);
    }
    let trimmed: String = line.chars().take(width).collect();
    Line::from(Span::styled(trimmed, Style::default().fg(Color::DarkGray)))
}

fn line_is_blank(line: &Line) -> bool {
    line.spans.iter().all(|span| span.content.trim().is_empty())
}

pub(crate) fn render_user_bubble(text: &str, width: usize) -> Vec<Line<'static>> {
    let bubble_width = (width * USER_MAX_WIDTH_PERCENT / 100).max(20);
    let style = Style::default().bg(USER_BUBBLE_BG).fg(Color::White);
    wrap_text(text, bubble_width)
        .into_iter()
        .map(|line| Line::from(Span::styled(format!(" {line} "), style)))
        .collect()
}

pub(crate) fn render_styled_prefixed(prefix: &str, text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    wrap_prefixed(prefix, text, width)
        .into_iter()
        .map(|line| Line::from(Span::styled(line, style)))
        .collect()
}

pub(crate) fn intro_text(state: &ChatUiState) -> String {
    format!(
        "Welcome to Agnes Chat.\n\nAsk for code changes, image understanding, image generation, async video tasks, asset/history lookup, or skill loading. Use /image <URL, path, or Base64> to attach an image, then send your question. /images lists queued images.\n\nTab completes slash commands. /retry or Ctrl-R retries the previous request without adding another user message to the session. Ctrl-T toggles thinking visibility. Ctrl-E toggles tool details. Current approval mode: {}.",
        state.telemetry.approval_label
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::chat::model::ChatLineKind;
    use crate::ui::chat::state::ChatUiState;

    fn transcript_line_texts(state: &ChatUiState, width: usize) -> Vec<String> {
        build_transcript_lines(state, width)
            .into_iter()
            .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>())
            .collect()
    }

    #[test]
    fn thinking_is_collapsed_by_default_and_can_expand() {
        let mut state = ChatUiState::default();
        state.push_thinking_delta("private reasoning details");

        assert!(
            transcript_line_texts(&state, 80)
                .iter()
                .any(|line| line.contains("Thinking hidden"))
        );
        assert!(
            !transcript_line_texts(&state, 80)
                .iter()
                .any(|line| line.contains("private reasoning details"))
        );

        state.toggle_thinking();

        assert!(
            transcript_line_texts(&state, 80)
                .iter()
                .any(|line| line.contains("private reasoning details"))
        );
    }

    #[test]
    fn tools_are_collapsed_by_default_and_can_expand() {
        let mut state = ChatUiState::default();
        state.push_line(ChatLineKind::Tool, "agnes_submit_video({\"prompt\":\"test\"})");

        assert!(
            transcript_line_texts(&state, 80)
                .iter()
                .any(|line| line.contains("Tool activity hidden"))
        );
        assert!(
            !transcript_line_texts(&state, 80)
                .iter()
                .any(|line| line.contains("agnes_submit_video"))
        );

        state.toggle_tools();

        assert!(
            transcript_line_texts(&state, 80)
                .iter()
                .any(|line| line.contains("agnes_submit_video"))
        );
    }

    #[test]
    fn user_bubble_is_left_aligned_with_background() {
        let lines = render_user_bubble("hello", 40);
        let bubble = lines
            .iter()
            .find(|line| line.spans.iter().any(|span| span.content.contains("hello")))
            .expect("bubble line");
        assert_eq!(bubble.spans.len(), 1);
        assert_eq!(bubble.spans[0].style.bg, Some(USER_BUBBLE_BG));
        assert!(bubble.spans[0].content.contains("hello"));
    }

    #[test]
    fn transcript_uses_dashed_dividers_between_turns() {
        let mut state = ChatUiState::default();
        state.push_user("hello");
        state.push_line(ChatLineKind::Thinking, "hmm");
        state.push_line(ChatLineKind::Assistant, "world");
        state.push_user("again");

        let lines = transcript_line_texts(&state, 40);
        let divider_count = lines.iter().filter(|line| line.contains('─')).count();
        assert_eq!(divider_count, 1, "only divider between turns: {lines:?}");
    }

    #[test]
    fn thinking_and_tools_skip_dividers_within_a_turn() {
        let mut state = ChatUiState { tools_expanded: true, thinking_expanded: true, ..ChatUiState::default() };
        state.push_user("hello");
        state.push_line(ChatLineKind::Thinking, "plan");
        state.push_line(ChatLineKind::Tool, "→ run");
        state.push_line(ChatLineKind::Assistant, "done");

        let lines = transcript_line_texts(&state, 40);
        assert_eq!(lines.iter().filter(|line| line.contains('─')).count(), 0);
    }

    #[test]
    fn intro_text_explains_the_interface() {
        let state = ChatUiState::default();

        assert!(intro_text(&state).contains("Tab completes slash commands"));
        assert!(intro_text(&state).contains("Ctrl-T"));
    }
}
