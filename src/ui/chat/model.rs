use pi_agent::AgentEvent;
use pi_ai::Message;
use serde_json::Value;
use tokio::sync::oneshot;

use crate::agent::chat::ChatOverrides;
use crate::cli::ChatArgs;
use pi_agent::PermissionDecision;

#[derive(Debug, Clone)]
pub struct ChatUiOptions {
    pub resume: Option<String>,
    pub auto: bool,
    pub max_turns: u32,
    pub overrides: ChatOverrides,
}

impl From<ChatArgs> for ChatUiOptions {
    fn from(args: ChatArgs) -> Self {
        let overrides = args.overrides();
        Self { resume: args.resume, auto: args.auto, max_turns: args.max_turns, overrides }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatLineKind {
    User,
    Assistant,
    Thinking,
    Tool,
    Status,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    pub kind: ChatLineKind,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum RunPhase {
    #[default]
    Idle,
    Waiting,
    Thinking,
    Responding,
    Tool(String),
}

#[derive(Debug, Clone)]
pub(crate) struct RetryRequest {
    pub(crate) input: String,
    pub(crate) history: Vec<Message>,
}

pub(crate) struct ApprovalRequest {
    pub(crate) tool_name: String,
    pub(crate) args: Value,
    pub(crate) respond_to: oneshot::Sender<PermissionDecision>,
}

pub(crate) enum UiEvent {
    Agent(AgentEvent),
    Approval(ApprovalRequest),
    AutoApproved { tool_name: String },
    Finished(std::result::Result<(Vec<Message>, bool), String>),
}
