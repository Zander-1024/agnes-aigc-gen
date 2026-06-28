use ratatui::text::Line;

pub(crate) fn wrap_prefixed(prefix: &str, text: &str, width: usize) -> Vec<String> {
    let available = width.saturating_sub(prefix.chars().count()).max(8);
    let wrapped = wrap_text(text, available);
    let indent = " ".repeat(prefix.chars().count());
    wrapped
        .into_iter()
        .enumerate()
        .map(|(idx, line)| {
            if idx == 0 {
                format!("{prefix}{line}")
            } else {
                format!("{indent}{line}")
            }
        })
        .collect()
}

pub(crate) fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for raw_line in text.lines() {
        if raw_line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in raw_line.split_whitespace() {
            let word_len = word.chars().count();
            let line_len = line.chars().count();
            if line.is_empty() {
                if word_len <= width {
                    line.push_str(word);
                } else {
                    out.extend(split_long_word(word, width));
                }
            } else if line_len + 1 + word_len <= width {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(line);
                line = String::new();
                if word_len <= width {
                    line.push_str(word);
                } else {
                    out.extend(split_long_word(word, width));
                }
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    out
}

pub(crate) fn wrap_styled_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if line.width() <= width {
        return vec![line];
    }
    let plain: String = line.spans.iter().map(|span| span.content.as_ref()).collect();
    wrap_text(&plain, width).into_iter().map(Line::from).collect()
}

fn split_long_word(word: &str, width: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in word.chars() {
        if current.chars().count() >= width {
            chunks.push(current);
            current = String::new();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_long_text_to_available_width() {
        let wrapped = wrap_text("one two three four five", 9);

        assert_eq!(wrapped, vec!["one two", "three", "four five"]);
    }
}
