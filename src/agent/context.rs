use pi_ai::{Content, Message};

pub const AUTO_COMPRESS_THRESHOLD_PERCENT: u64 = 90;
pub const PRESERVE_RECENT_MESSAGES: usize = 12;
const SUMMARY_SNIPPET_CHARS: usize = 240;
const TOOL_SNIPPET_CHARS: usize = 120;

#[derive(Debug, Clone)]
pub struct CompressionResult {
    pub messages: Vec<Message>,
    pub removed_count: usize,
    pub tokens_before: u64,
    pub tokens_after: u64,
}

pub fn estimate_tokens(messages: &[Message], system_prompt: &str) -> u64 {
    let mut chars = system_prompt.len() as u64;
    for message in messages {
        chars += message_text(message).len() as u64;
        chars += 8;
    }
    (chars / 4).max(1)
}

pub fn usage_percent(used: u64, limit: u32) -> u64 {
    if limit == 0 {
        return 0;
    }
    ((used.saturating_mul(100)) / u64::from(limit)).min(100)
}

pub fn should_auto_compress(used: u64, limit: u32) -> bool {
    usage_percent(used, limit) >= AUTO_COMPRESS_THRESHOLD_PERCENT
}

pub fn compress_messages(messages: &[Message], preserve_recent: usize) -> Option<CompressionResult> {
    if messages.len() <= preserve_recent.saturating_add(1) {
        return None;
    }
    let split_at = messages.len().saturating_sub(preserve_recent);
    let (older, recent) = messages.split_at(split_at);
    if older.is_empty() {
        return None;
    }

    let summary = build_compression_summary(older);
    let compressed = Message::user_text(summary);
    let mut out = vec![compressed];
    out.extend(recent.iter().cloned());

    let tokens_before = estimate_tokens(messages, "");
    let tokens_after = estimate_tokens(&out, "");
    Some(CompressionResult { messages: out, removed_count: older.len(), tokens_before, tokens_after })
}

pub fn maybe_compress(
    messages: &mut Vec<Message>,
    limit: u32,
    system_prompt: &str,
    force: bool,
) -> Option<CompressionResult> {
    let tokens_before = estimate_tokens(messages, system_prompt);
    if !force && !should_auto_compress(tokens_before, limit) {
        return None;
    }
    let result = compress_messages(messages, PRESERVE_RECENT_MESSAGES)?;
    let tokens_after = estimate_tokens(&result.messages, system_prompt);
    *messages = result.messages.clone();
    Some(CompressionResult {
        messages: result.messages,
        removed_count: result.removed_count,
        tokens_before,
        tokens_after,
    })
}

fn build_compression_summary(older: &[Message]) -> String {
    let mut parts = vec![
        "## Compressed conversation history".into(),
        format!(
            "Summarized {} earlier messages removed to stay within context limits.",
            older.len()
        ),
        String::new(),
    ];
    for (index, message) in older.iter().enumerate() {
        parts.push(format!("### Message {} ({})", index + 1, message_role(message)));
        parts.push(truncate_chars(&message_text(message), SUMMARY_SNIPPET_CHARS));
        parts.push(String::new());
    }
    parts.join("\n")
}

fn message_role(message: &Message) -> &'static str {
    match message {
        Message::User { .. } => "user",
        Message::Assistant(_) => "assistant",
        Message::ToolResult(_) => "tool",
    }
}

fn message_text(message: &Message) -> String {
    match message {
        Message::User { content, .. } => content
            .iter()
            .map(content_snippet)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Message::Assistant(assistant) => assistant
            .content
            .iter()
            .map(content_snippet)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Message::ToolResult(result) => {
            let body = result
                .content
                .iter()
                .map(content_snippet)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "[tool {}] {}",
                result.tool_name,
                truncate_chars(&body, TOOL_SNIPPET_CHARS)
            )
        }
    }
}

fn content_snippet(content: &Content) -> String {
    match content {
        Content::Text { text } => text.clone(),
        Content::Thinking { thinking, .. } => format!("[thinking] {}", truncate_chars(thinking, 80)),
        Content::ToolCall { name, .. } => format!("[tool call {name}]"),
        Content::Image { .. } => "[image]".into(),
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    format!(
        "{}...",
        text.chars().take(max_chars.saturating_sub(3)).collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_scales_with_content() {
        let short = estimate_tokens(&[Message::user_text("hi")], "system");
        let long = estimate_tokens(&[Message::user_text("word ".repeat(400))], "system");
        assert!(long > short);
    }

    #[test]
    fn compress_keeps_recent_messages() {
        let messages: Vec<Message> = (0..20)
            .map(|idx| Message::user_text(format!("message {idx} {}", "x".repeat(200))))
            .collect();
        let result = compress_messages(&messages, 4).unwrap();

        assert_eq!(result.removed_count, 16);
        assert_eq!(result.messages.len(), 5);
        assert!(message_text(&result.messages[0]).contains("Compressed conversation"));
        assert!(message_text(result.messages.last().unwrap()).contains("message 19"));
    }

    #[test]
    fn auto_compress_triggers_at_ninety_percent() {
        assert!(!should_auto_compress(89, 100));
        assert!(should_auto_compress(90, 100));
    }

    #[test]
    fn maybe_compress_noop_below_threshold() {
        let mut messages = vec![Message::user_text("small")];
        assert!(maybe_compress(&mut messages, 256_000, "", false).is_none());
    }
}
