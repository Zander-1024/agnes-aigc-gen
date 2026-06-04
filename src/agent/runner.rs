use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use futures::StreamExt;
use pi_agent::{AgentEvent, AgentTool, AgentToolResult, PermissionDecision, PermissionPolicy, tool_def};
use pi_ai::{AssistantMessageEvent, Content, Context, Message, StopReason, ToolResultMessage};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::agent::agnes_stream::{AgnesStreamOptions, stream_agnes};
use crate::agent::chat::ChatRuntimeConfig;

#[derive(Clone)]
pub struct AgnesAgentConfig {
    pub runtime: ChatRuntimeConfig,
    pub tools: Vec<Arc<dyn AgentTool>>,
    pub system_prompt: String,
    pub max_turns: u32,
    pub permission: Arc<dyn PermissionPolicy>,
}

pub struct AgnesAgentRun {
    pub messages: Vec<Message>,
    pub stopped_at_turn_limit: bool,
}

pub async fn run_agnes_agent_with_history(
    config: &AgnesAgentConfig,
    mut messages: Vec<Message>,
    events: Option<mpsc::UnboundedSender<AgentEvent>>,
) -> Result<AgnesAgentRun> {
    if let Some(last) = messages.last().cloned() {
        emit(&events, AgentEvent::UserMessage { message: last });
    }
    emit(&events, AgentEvent::AgentStart);

    let tool_index: HashMap<String, Arc<dyn AgentTool>> = config
        .tools
        .iter()
        .map(|tool| (tool.name().to_string(), tool.clone()))
        .collect();
    let tool_defs: Vec<pi_ai::Tool> = config.tools.iter().map(|tool| tool_def(tool.as_ref())).collect();
    let mut session_allowed: HashSet<String> = HashSet::new();
    let mut turn = 0u32;

    while turn < config.max_turns {
        turn += 1;
        emit(&events, AgentEvent::TurnStart);

        let ctx = Context {
            system_prompt: Some(config.system_prompt.clone()),
            messages: messages.clone(),
            tools: tool_defs.clone(),
        };
        let stream_options = AgnesStreamOptions {
            stream: config.runtime.stream.clone(),
            enable_thinking: config.runtime.enable_thinking,
        };
        let mut stream = stream_agnes(&config.runtime.model, &ctx, &stream_options).await?;
        let mut final_message = None;
        let mut stop = StopReason::Stop;

        while let Some(event) = stream.next().await {
            match event? {
                AssistantMessageEvent::Done { reason, message } => {
                    stop = reason;
                    final_message = Some(message);
                    break;
                }
                AssistantMessageEvent::Error { error, .. } => {
                    let message = error.error_message.clone().unwrap_or_else(|| "provider error".into());
                    anyhow::bail!(message);
                }
                AssistantMessageEvent::TextDelta { delta, .. } => {
                    emit(&events, AgentEvent::TextDelta { delta });
                }
                AssistantMessageEvent::ThinkingDelta { delta, .. } => {
                    emit(&events, AgentEvent::ThinkingDelta { delta });
                }
                _ => {}
            }
        }

        let message = final_message.context("provider stream produced no terminal event")?;
        let assistant_message = Message::Assistant(message.clone());
        messages.push(assistant_message.clone());
        emit(&events, AgentEvent::AssistantMessage { message: assistant_message });

        let tool_calls: Vec<(String, String, Value)> = message
            .content
            .iter()
            .filter_map(|content| match content {
                Content::ToolCall { id, name, arguments } => Some((id.clone(), name.clone(), arguments.clone())),
                _ => None,
            })
            .collect();

        if tool_calls.is_empty() || stop != StopReason::ToolUse {
            emit(&events, AgentEvent::TurnEnd);
            emit(&events, AgentEvent::AgentEnd { messages: messages.clone() });
            return Ok(AgnesAgentRun { messages, stopped_at_turn_limit: false });
        }

        let mut any_terminate = !tool_calls.is_empty();
        for (id, name, args) in tool_calls {
            let tool_obj = tool_index.get(&name);
            let needs_permission =
                tool_obj.map(|tool| tool.requires_permission()).unwrap_or(false) && !session_allowed.contains(&name);
            if needs_permission {
                match config.permission.check(&name, &args).await {
                    PermissionDecision::Allow => {}
                    PermissionDecision::AllowSession => {
                        session_allowed.insert(name.clone());
                    }
                    PermissionDecision::Deny { reason } => {
                        emit(
                            &events,
                            AgentEvent::PermissionDenied { tool_name: name.clone(), reason: reason.clone() },
                        );
                        messages.push(Message::ToolResult(ToolResultMessage {
                            tool_call_id: id,
                            tool_name: name,
                            content: vec![Content::text(format!("permission denied: {reason}"))],
                            is_error: true,
                            timestamp: pi_ai::now_ms(),
                        }));
                        any_terminate = false;
                        continue;
                    }
                }
            }

            emit(
                &events,
                AgentEvent::ToolExecutionStart {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    args: args.clone(),
                },
            );
            let (content, is_error, terminate) = execute_tool(tool_obj, &id, args).await;
            if !terminate {
                any_terminate = false;
            }
            emit(
                &events,
                AgentEvent::ToolExecutionEnd {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    is_error,
                    content: content.clone(),
                },
            );
            messages.push(Message::ToolResult(ToolResultMessage {
                tool_call_id: id,
                tool_name: name,
                content,
                is_error,
                timestamp: pi_ai::now_ms(),
            }));
        }

        emit(&events, AgentEvent::TurnEnd);
        if any_terminate {
            break;
        }
    }

    emit(&events, AgentEvent::AgentEnd { messages: messages.clone() });
    Ok(AgnesAgentRun { messages, stopped_at_turn_limit: true })
}

async fn execute_tool(tool_obj: Option<&Arc<dyn AgentTool>>, id: &str, args: Value) -> (Vec<Content>, bool, bool) {
    match tool_obj {
        Some(tool) => match tool.execute(id, args).await {
            Ok(AgentToolResult { content, details: _, terminate }) => (content, false, terminate),
            Err(err) => (vec![Content::text(format!("tool error: {err}"))], true, false),
        },
        None => (vec![Content::text("unknown tool".to_string())], true, false),
    }
}

fn emit(sink: &Option<mpsc::UnboundedSender<AgentEvent>>, event: AgentEvent) {
    if let Some(sink) = sink {
        let _ = sink.send(event);
    }
}
