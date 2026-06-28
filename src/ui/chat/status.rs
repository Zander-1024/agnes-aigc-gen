use super::model::RunPhase;
use super::state::ChatUiState;

pub(crate) fn approval_label_for(auto_mode: bool) -> String {
    if auto_mode {
        "auto, dangerous calls still reviewed".into()
    } else {
        "review".into()
    }
}

pub(crate) fn format_tokens(tokens: u32) -> String {
    if tokens.is_multiple_of(1024) {
        format!("{}k", tokens / 1024)
    } else {
        tokens.to_string()
    }
}

pub(crate) fn format_u64_tokens(tokens: u64) -> String {
    format_tokens(tokens.min(u64::from(u32::MAX)) as u32)
}

pub(crate) fn activity_status(state: &ChatUiState) -> String {
    if state.approval.is_active() {
        return format!(
            "approval pending: {}",
            state
                .approval
                .pending
                .as_ref()
                .map(|request| request.tool_name.as_str())
                .unwrap_or("tool")
        );
    }
    if !state.busy {
        return "idle".into();
    }
    match &state.run_phase {
        RunPhase::Idle => "running".into(),
        RunPhase::Waiting => "waiting for model".into(),
        RunPhase::Thinking => "thinking".into(),
        RunPhase::Responding => "responding".into(),
        RunPhase::Tool(name) => format!("tool: {name}"),
    }
}

pub(crate) fn cache_hit_label(state: &ChatUiState) -> String {
    let t = &state.telemetry;
    if t.last_prompt_tokens == 0 {
        return "— (after first response)".into();
    }
    let pct = t.last_cache_read.saturating_mul(100) / t.last_prompt_tokens.max(1);
    format!(
        "{} / {} ({}%)",
        format_u64_tokens(t.last_cache_read),
        format_u64_tokens(t.last_prompt_tokens),
        pct
    )
}

pub(crate) fn status_line(state: &ChatUiState) -> String {
    format!(
        "{}  |  approval: {}  |  {}  |  Ctrl-Q quit",
        state.status,
        state.telemetry.approval_label,
        activity_status(state)
    )
}

pub(crate) fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        format!(
            "{}...",
            text.chars().take(max_chars.saturating_sub(3)).collect::<String>()
        )
    }
}
