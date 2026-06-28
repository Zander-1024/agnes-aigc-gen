/// Parse `description` from SKILL.md YAML front matter (folded block + scalar subset).
pub(crate) fn skill_description_from_markdown(text: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()? != "---" {
        return first_body_line(text);
    }
    while let Some(line) = lines.next() {
        if line == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix("description:") {
            let value = value.trim();
            if matches!(value, ">-" | ">" | "|-" | "|") {
                let folded = collect_folded_yaml_value(&mut lines);
                if !folded.is_empty() {
                    return Some(folded);
                }
            } else if let Some(description) = yaml_scalar(value) {
                return Some(description);
            }
        }
    }
    let body = lines.collect::<Vec<_>>().join("\n");
    first_body_line(&body)
}

fn collect_folded_yaml_value<'a>(lines: &mut impl Iterator<Item = &'a str>) -> String {
    let mut parts = Vec::new();
    for line in lines.by_ref() {
        if line == "---" {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            parts.push(line.trim());
        } else {
            break;
        }
    }
    parts.join(" ")
}

fn yaml_scalar(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('"').trim_matches('\'').trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn first_body_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && *line != "---")
        .map(|line| line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_description_parses_folded_front_matter() {
        let text = "---\nname: demo\ndescription: >-\n  Does one thing.\n  Also does another.\n---\n# Demo\n";

        assert_eq!(
            skill_description_from_markdown(text).as_deref(),
            Some("Does one thing. Also does another.")
        );
    }
}
