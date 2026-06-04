use std::path::{Path, PathBuf};

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Allow,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalMode {
    Review,
    Auto,
}

#[derive(Debug, Clone)]
pub struct ApprovalPolicy {
    mode: ApprovalMode,
    workspace: PathBuf,
}

impl ApprovalPolicy {
    pub fn default_review(workspace: PathBuf) -> Self {
        Self { mode: ApprovalMode::Review, workspace }
    }

    pub fn auto(workspace: PathBuf) -> Self {
        Self { mode: ApprovalMode::Auto, workspace }
    }

    pub fn classify(&self, tool_name: &str, args: &Value) -> ApprovalDecision {
        if is_safe_tool(tool_name) {
            return if path_args_stay_in_workspace(tool_name, args, &self.workspace) {
                ApprovalDecision::Allow
            } else {
                ApprovalDecision::Review
            };
        }

        if tool_name == "bash" {
            if bash_command_is_dangerous(args) {
                return ApprovalDecision::Review;
            }
            return match self.mode {
                ApprovalMode::Review => ApprovalDecision::Review,
                ApprovalMode::Auto => ApprovalDecision::Allow,
            };
        }

        if is_write_tool(tool_name) && !path_args_stay_in_workspace(tool_name, args, &self.workspace) {
            return ApprovalDecision::Review;
        }

        if is_media_tool(tool_name) {
            return match self.mode {
                ApprovalMode::Review => ApprovalDecision::Review,
                ApprovalMode::Auto => ApprovalDecision::Allow,
            };
        }

        if is_review_tool(tool_name) {
            return match self.mode {
                ApprovalMode::Review => ApprovalDecision::Review,
                ApprovalMode::Auto => ApprovalDecision::Allow,
            };
        }

        ApprovalDecision::Review
    }
}

fn is_safe_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "read"
            | "ls"
            | "grep"
            | "glob"
            | "todo"
            | "agnes_task_list"
            | "agnes_task_show"
            | "agnes_asset_list"
            | "agnes_asset_show"
            | "agnes_history_list"
            | "agnes_history_show"
            | "load_skill"
    )
}

fn is_write_tool(tool_name: &str) -> bool {
    matches!(tool_name, "write" | "edit")
}

fn is_media_tool(tool_name: &str) -> bool {
    matches!(tool_name, "agnes_generate_image" | "agnes_submit_video")
}

fn is_review_tool(tool_name: &str) -> bool {
    matches!(tool_name, "write" | "edit" | "web_fetch" | "agnes_task_wait")
}

fn path_args_stay_in_workspace(tool_name: &str, args: &Value, workspace: &Path) -> bool {
    let Some(path) = path_arg(tool_name, args) else {
        return true;
    };
    path_stays_in_workspace(path, workspace)
}

fn path_arg<'a>(tool_name: &str, args: &'a Value) -> Option<&'a str> {
    match tool_name {
        "read" | "write" | "edit" => args.get("path").and_then(Value::as_str),
        "ls" | "grep" | "glob" => args
            .get("path")
            .or_else(|| args.get("directory"))
            .or_else(|| args.get("cwd"))
            .and_then(Value::as_str),
        _ => None,
    }
}

fn path_stays_in_workspace(raw: &str, workspace: &Path) -> bool {
    let path = Path::new(raw);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };
    let mut depth = 0i32;
    for component in resolved.components() {
        match component {
            std::path::Component::ParentDir => depth -= 1,
            std::path::Component::Normal(_) => depth += 1,
            _ => {}
        }
    }
    depth >= 0 && resolved.starts_with(workspace)
}

fn bash_command_is_dangerous(args: &Value) -> bool {
    let Some(command) = args.get("command").and_then(Value::as_str) else {
        return true;
    };
    let normalized = command.to_lowercase();
    let compact = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let dangerous_fragments = [
        "git reset --hard",
        "git clean",
        "git push --force",
        "git push -f",
        "git tag -f",
        "rm -rf",
        "sudo ",
        "chmod ",
        "chown ",
        "cargo publish",
        "npm publish",
        "pnpm publish",
        "yarn publish",
        "gh release",
        "git push origin v",
    ];
    dangerous_fragments.iter().any(|fragment| compact.contains(fragment))
        || compact.contains("curl") && (compact.contains("| sh") || compact.contains("| bash"))
        || compact.contains("wget") && (compact.contains("| sh") || compact.contains("| bash"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn read_inside_workspace_is_safe() {
        let workspace = PathBuf::from("/repo");
        let policy = ApprovalPolicy::default_review(workspace);

        let decision = policy.classify("read", &json!({"path": "/repo/src/main.rs"}));

        assert_eq!(decision, ApprovalDecision::Allow);
    }

    #[test]
    fn write_requires_review() {
        let workspace = PathBuf::from("/repo");
        let policy = ApprovalPolicy::default_review(workspace);

        let decision = policy.classify("write", &json!({"path": "/repo/src/main.rs"}));

        assert_eq!(decision, ApprovalDecision::Review);
    }

    #[test]
    fn dangerous_bash_requires_review_even_in_auto_mode() {
        let workspace = PathBuf::from("/repo");
        let policy = ApprovalPolicy::auto(workspace);

        let decision = policy.classify("bash", &json!({"command": "git reset --hard HEAD"}));

        assert_eq!(decision, ApprovalDecision::Review);
    }
}
