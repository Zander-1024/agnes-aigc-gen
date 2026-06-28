use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::agent::context::usage_percent;

use super::approval::render_approval_actions;
use super::input::{build_input_line, input_viewport};
use super::scroll::slice_transcript_view;
use super::state::ChatUiState;
use super::status::{activity_status, cache_hit_label, format_tokens, format_u64_tokens, status_line};
use super::transcript::{build_intro_lines, build_transcript_lines};
use super::wrap::wrap_text;

pub(crate) fn render(frame: &mut ratatui::Frame, state: &mut ChatUiState) {
    let approval_active = state.approval.is_active();
    let completion_height = if state.input.completion_hint.is_some() { 2 } else { 0 };
    let mut constraints = vec![Constraint::Length(9), Constraint::Min(3)];
    if approval_active {
        constraints.push(Constraint::Percentage(32));
    }
    if completion_height > 0 {
        constraints.push(Constraint::Length(completion_height));
    }
    constraints.push(Constraint::Length(3));
    constraints.push(Constraint::Length(1));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(frame.area());

    let mut idx = 0;
    render_header(frame, chunks[idx], state);
    idx += 1;
    render_transcript(frame, chunks[idx], state);
    idx += 1;
    if approval_active {
        render_approval(frame, chunks[idx], state);
        idx += 1;
    }
    if completion_height > 0 {
        render_completion(frame, chunks[idx], state);
        idx += 1;
    }
    let input_block = Block::default().title(input_title(state)).borders(Borders::TOP);
    let input_inner = input_block.inner(chunks[idx]);
    let prompt = "> ";
    let max_width = input_inner.width.max(1) as usize;
    let (display, cursor_col) = input_viewport(prompt, &state.input.buffer, state.input.cursor, max_width);
    let input_line = build_input_line(&display, cursor_col, state.input.blink_on);
    let input = Paragraph::new(input_line).block(input_block);
    frame.render_widget(input, chunks[idx]);
    idx += 1;
    frame.render_widget(Paragraph::new(status_line(state)), chunks[idx]);
}

fn input_title(state: &ChatUiState) -> Line<'static> {
    if state.busy {
        Line::from(vec![
            Span::raw("Message "),
            Span::styled(
                format!("({})", activity_status(state)),
                Style::default().fg(Color::Yellow),
            ),
        ])
    } else {
        Line::from("Message")
    }
}

fn render_header(frame: &mut ratatui::Frame, area: Rect, state: &ChatUiState) {
    let thinking_fold = if state.thinking_expanded { "expanded" } else { "folded" };
    let tool_fold = if state.tools_expanded { "expanded" } else { "folded" };
    let t = &state.telemetry;
    let context_used = t.last_prompt_tokens.max(t.estimated_session_tokens);
    let context_pct = usage_percent(context_used, t.context_tokens);
    let context_style = if context_pct >= 90 {
        Style::default().fg(Color::Red)
    } else if context_pct >= 75 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Green)
    };
    let cache_label = cache_hit_label(state);
    let activity = activity_status(state);
    let activity_style = if state.busy {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let text = vec![
        Line::from(vec![
            Span::styled(">_ Agnes Chat", Style::default().fg(Color::Cyan)),
            Span::raw("  PI agent for Agnes media + coding"),
        ]),
        Line::from(vec![
            Span::styled("model: ", Style::default().fg(Color::DarkGray)),
            Span::raw(t.model_label.as_str()),
            Span::raw("    "),
            Span::styled("thinking: ", Style::default().fg(Color::DarkGray)),
            Span::raw(if t.thinking_enabled { "on" } else { "off" }),
            Span::raw(format!(" ({thinking_fold})")),
            Span::raw("    "),
            Span::styled("approval: ", Style::default().fg(Color::DarkGray)),
            Span::raw(t.approval_label.as_str()),
        ]),
        Line::from(vec![
            Span::styled("context: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "{}/{} ({}%)",
                    format_u64_tokens(context_used),
                    format_tokens(t.context_tokens),
                    context_pct
                ),
                context_style,
            ),
            Span::raw("    "),
            Span::styled("output: ", Style::default().fg(Color::DarkGray)),
            Span::raw(format_tokens(t.max_output_tokens)),
        ]),
        Line::from(vec![
            Span::styled("cache hit: ", Style::default().fg(Color::DarkGray)),
            Span::raw(cache_label),
            Span::raw("    "),
            Span::styled("tools: ", Style::default().fg(Color::DarkGray)),
            Span::raw(tool_fold),
        ]),
        Line::from(vec![
            Span::styled("activity: ", Style::default().fg(Color::DarkGray)),
            Span::styled(activity, activity_style),
        ]),
        Line::from(vec![
            Span::styled("directory: ", Style::default().fg(Color::DarkGray)),
            Span::raw(state.telemetry.directory_label.as_str()),
        ]),
        Line::from(vec![
            Span::styled("Tip: ", Style::default().fg(Color::Yellow)),
            Span::raw("Shift-Tab approval. ←/→ approve. Tab slash. Ctrl-R retry. PgUp/PgDn scroll. /compress context."),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_transcript(frame: &mut ratatui::Frame, area: Rect, state: &mut ChatUiState) {
    let inner_width = area.width.saturating_sub(2).max(20) as usize;
    let inner_height = area.height.saturating_sub(2).max(1) as usize;
    state.transcript.viewport = inner_height as u16;

    let all_lines = if state.lines.is_empty() {
        build_intro_lines(state, inner_width)
    } else {
        build_transcript_lines(state, inner_width)
    };
    let visible = slice_transcript_view(all_lines, inner_height, state.transcript.from_bottom);

    frame.render_widget(
        Paragraph::new(visible)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(Span::styled(" Chat ", Style::default().fg(Color::Cyan))),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_approval(frame: &mut ratatui::Frame, area: Rect, state: &mut ChatUiState) {
    let (tool_name, args) = match &state.approval.pending {
        Some(request) => (request.tool_name.clone(), request.args.clone()),
        None => return,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(
            " Approval required ",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(inner);

    let content_width = chunks[0].width.saturating_sub(2).max(20) as usize;
    let content_height = chunks[0].height.saturating_sub(0).max(1) as usize;
    state.approval.viewport = content_height as u16;

    let args_pretty = serde_json::to_string_pretty(&args).unwrap_or_else(|_| args.to_string());
    let mut content_lines = vec![
        Line::from(vec![
            Span::styled("Tool  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                tool_name,
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled("Arguments", Style::default().fg(Color::DarkGray))),
    ];
    for segment in wrap_text(&args_pretty, content_width) {
        content_lines.push(Line::from(Span::styled(segment, Style::default().fg(Color::Gray))));
    }
    let visible_content = slice_transcript_view(content_lines, content_height, state.approval.scroll_from_bottom);
    frame.render_widget(Paragraph::new(visible_content).wrap(Wrap { trim: false }), chunks[0]);

    let action_line = render_approval_actions(state.approval.choice);
    let hint = Line::from(Span::styled(
        "↑/↓ scroll args  ←/→ select  Enter confirm  y/a/n shortcuts",
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(Paragraph::new(vec![action_line, hint]), chunks[1]);
}

fn render_completion(frame: &mut ratatui::Frame, area: Rect, state: &ChatUiState) {
    let text = state.input.completion_hint.as_deref().unwrap_or("");
    frame.render_widget(
        Paragraph::new(text)
            .style(Style::default().fg(Color::DarkGray))
            .wrap(Wrap { trim: true }),
        area,
    );
}
