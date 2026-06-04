use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use clap::Args;
use pi_agent::{AgentEvent, PermissionDecision, PermissionPolicy};
use pi_ai::Message;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::agent::approval::{ApprovalDecision, ApprovalPolicy};
use crate::agent::chat::{ChatOverrides, build_agent_config};
use crate::agent::runner::run_agnes_agent_with_history;
use crate::agent::session::{ChatSession, load_session, save_session};
use crate::config::{AppConfig, parse_token_limit};

#[derive(Args, Debug, Clone)]
pub struct ChatArgs {
    /// One-shot prompt. Omit to launch the TUI.
    #[arg(short, long)]
    pub prompt: Option<String>,

    /// Resume a saved chat session id.
    #[arg(long)]
    pub resume: Option<String>,

    /// Auto-approve non-dangerous tool calls.
    #[arg(long)]
    pub auto: bool,

    /// Maximum agent turns per user message.
    #[arg(long, default_value_t = 32)]
    pub max_turns: u32,

    /// Enable thinking for this run.
    #[arg(long, conflicts_with = "no_thinking")]
    pub thinking: bool,

    /// Disable thinking for this run.
    #[arg(long = "no-thinking", conflicts_with = "thinking")]
    pub no_thinking: bool,

    /// Override chat context length, e.g. 256k or 262144.
    #[arg(long = "context-tokens", value_parser = parse_token_arg)]
    pub context_tokens: Option<u32>,

    /// Override max output tokens, e.g. 64k or 65536.
    #[arg(long = "max-output-tokens", value_parser = parse_token_arg)]
    pub max_output_tokens: Option<u32>,
}

impl ChatArgs {
    pub fn overrides(&self) -> ChatOverrides {
        ChatOverrides {
            thinking: if self.thinking {
                Some(true)
            } else if self.no_thinking {
                Some(false)
            } else {
                None
            },
            context_tokens: self.context_tokens,
            max_output_tokens: self.max_output_tokens,
        }
    }
}

pub fn run(args: ChatArgs) -> Result<()> {
    if args.prompt.is_none() {
        return crate::ui::chat::run(args.into());
    }
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(run_prompt(args))
}

async fn run_prompt(args: ChatArgs) -> Result<()> {
    let cfg = AppConfig::load()?;
    let workspace = std::env::current_dir()?;
    let permission = Arc::new(CliPermission::new(if args.auto {
        ApprovalPolicy::auto(workspace)
    } else {
        ApprovalPolicy::default_review(workspace)
    }));
    let agent = build_agent_config(&cfg, args.overrides(), args.max_turns, permission, &[])?;
    let mut session = match args.resume.as_deref() {
        Some(id) => load_session(id)?,
        None => ChatSession::new(&agent.runtime.model.id, agent.runtime.enable_thinking),
    };
    let prompt = args.prompt.clone().unwrap_or_default();
    let mut history = session.messages.clone();
    history.push(Message::user_text(prompt));

    let (tx, mut rx) = mpsc::unbounded_channel();
    let agent_for_task = agent.clone();
    let handle = tokio::spawn(async move { run_agnes_agent_with_history(&agent_for_task, history, Some(tx)).await });
    let mut stdout = std::io::stdout();
    while let Some(event) = rx.recv().await {
        print_agent_event(&mut stdout, event)?;
    }
    let run = handle.await??;
    if run.stopped_at_turn_limit {
        eprintln!("agent stopped at max turn limit ({})", args.max_turns);
    }
    session.model = agent.runtime.model.id;
    session.thinking = agent.runtime.enable_thinking;
    session.replace_messages(run.messages);
    save_session(&session)?;
    Ok(())
}

fn print_agent_event(stdout: &mut std::io::Stdout, event: AgentEvent) -> Result<()> {
    match event {
        AgentEvent::TextDelta { delta } => {
            write!(stdout, "{delta}")?;
            stdout.flush()?;
        }
        AgentEvent::AssistantMessage { .. } => {
            writeln!(stdout)?;
            stdout.flush()?;
        }
        AgentEvent::ThinkingDelta { .. } => {}
        AgentEvent::ToolExecutionStart { tool_name, args, .. } => {
            eprintln!("→ {tool_name}({args})");
        }
        AgentEvent::ToolExecutionEnd { tool_name, is_error, .. } => {
            eprintln!("← {tool_name} {}", if is_error { "error" } else { "ok" });
        }
        AgentEvent::PermissionDenied { tool_name, reason } => {
            eprintln!("✗ {tool_name} denied: {reason}");
        }
        _ => {}
    }
    Ok(())
}

struct CliPermission {
    policy: ApprovalPolicy,
    allowed_session: Mutex<std::collections::HashSet<String>>,
}

impl CliPermission {
    fn new(policy: ApprovalPolicy) -> Self {
        Self { policy, allowed_session: Mutex::new(Default::default()) }
    }
}

#[async_trait]
impl PermissionPolicy for CliPermission {
    async fn check(&self, tool_name: &str, args: &Value) -> PermissionDecision {
        if let Ok(allowed) = self.allowed_session.lock()
            && allowed.contains(tool_name)
        {
            return PermissionDecision::Allow;
        }
        if self.policy.classify(tool_name, args) == ApprovalDecision::Allow {
            return PermissionDecision::Allow;
        }
        prompt_permission(tool_name, args, &self.allowed_session)
    }
}

fn prompt_permission(
    tool_name: &str,
    args: &Value,
    allowed_session: &Mutex<std::collections::HashSet<String>>,
) -> PermissionDecision {
    let args_pretty = serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string());
    eprintln!("\nTool call requires approval: {tool_name}");
    eprintln!("{args_pretty}");
    eprint!("Allow? [y]es / [a]llow-session / [n]o: ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return PermissionDecision::Deny { reason: "approval prompt failed".into() };
    }
    match line.trim().to_lowercase().as_str() {
        "" | "y" | "yes" => PermissionDecision::Allow,
        "a" | "all" | "allow" | "session" => {
            if let Ok(mut allowed) = allowed_session.lock() {
                allowed.insert(tool_name.to_string());
            }
            PermissionDecision::AllowSession
        }
        _ => PermissionDecision::Deny { reason: "user denied".into() },
    }
}

fn parse_token_arg(value: &str) -> std::result::Result<u32, String> {
    parse_token_limit(value).map_err(|err| err.to_string())
}
