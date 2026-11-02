use std::collections::{BTreeMap, BTreeSet};

use async_stream::stream;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use pi_ai::{
    AssistantMessage, AssistantMessageEvent, AssistantMessageEventStream, Content, Context, Error, Message, Model,
    StopReason, StreamOptions, Usage,
};
use reqwest::Response;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::retry::{backoff_delay, is_retryable_http_status};

const CHAT_REQUEST_MAX_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Default)]
pub struct AgnesStreamOptions {
    pub stream: StreamOptions,
    pub enable_thinking: bool,
    pub request_body: Value,
    pub request_config_hash: String,
    pub request_layers: Vec<String>,
    pub record_requests: bool,
    pub session_id: Option<String>,
    pub turn: Option<u32>,
}

#[derive(Deserialize, Debug)]
struct Chunk {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<ChunkUsage>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ChunkChoice {
    #[serde(default)]
    delta: Option<ChunkDelta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct ChunkDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallDelta>,
}

#[derive(Deserialize, Debug)]
struct ToolCallDelta {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FunctionDelta>,
}

#[derive(Deserialize, Debug, Default)]
struct FunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct PromptTokensDetails {
    #[serde(default, alias = "cached_prompt_tokens")]
    cached_tokens: u64,
}

#[derive(Deserialize, Debug, Default)]
struct ChunkUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
    #[serde(default, alias = "cache_read_tokens")]
    cache_read_input_tokens: u64,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default)]
    input_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    args: String,
}

pub async fn stream_agnes(
    model: &Model,
    context: &Context,
    options: &AgnesStreamOptions,
) -> pi_ai::Result<AssistantMessageEventStream> {
    let api_key = options
        .stream
        .api_key
        .clone()
        .ok_or_else(|| Error::MissingApiKey("agnes".into()))?;
    let base_url = options
        .stream
        .base_url
        .clone()
        .unwrap_or_else(|| model.base_url.clone());
    let url = crate::api::endpoints::v1_endpoint(&base_url, "chat/completions")
        .map_err(|err| Error::Other(format!("invalid Agnes base URL: {err:#}")))?;
    let body = build_chat_body(model, context, options);
    crate::logging::log_request("POST", &url, Some(&body));
    log::debug!(
        "request config_hash={} layers={:?}",
        options.request_config_hash,
        options.request_layers
    );
    let mut request_log = if options.record_requests {
        Some(
            crate::request_log::RequestLog::start(
                "text",
                &model.id,
                &url,
                &options.request_config_hash,
                &options.request_layers,
                &body,
            )
            .map_err(|err| Error::Other(format!("record text request: {err:#}")))?,
        )
    } else {
        None
    };
    if let Some(log) = &request_log {
        log::debug!("request_id={}", log.id());
    }
    if let Some(log) = &mut request_log {
        log.context(options.session_id.as_deref(), options.turn)
            .map_err(|err| Error::Other(format!("record text request context: {err:#}")))?;
    }
    let cancel = options.stream.cancel.clone();
    let headers = options.stream.headers.clone();
    let client = reqwest::Client::new();

    let resp = match send_chat_request_with_retries(&client, &url, &api_key, &headers, &body, &mut request_log).await {
        Ok(response) => response,
        Err(error) => {
            let status = match &error {
                Error::ProviderError { status, .. } => Some(*status),
                _ => None,
            };
            record_request_failure(&mut request_log, &error.to_string(), status);
            return Err(error);
        }
    };
    if let Some(log) = &mut request_log
        && let Err(error) = log.response_status(resp.status().as_u16())
    {
        log::warn!("record text response status: {error:#}");
    }

    let api = model.api.clone();
    let provider = model.provider.clone();
    let model_id = model.id.clone();
    let event_stream = stream! {
        yield Ok(AssistantMessageEvent::Start);

        let mut sse = resp.bytes_stream().eventsource();
        let mut text_buf = String::new();
        let mut thinking_buf = String::new();
        let mut text_started = false;
        let mut thinking_started = false;
        let mut next_content_index: usize = 0;
        let mut tool_calls: BTreeMap<usize, PartialToolCall> = BTreeMap::new();
        let mut tool_started: BTreeSet<usize> = BTreeSet::new();
        let mut stop = StopReason::Stop;
        let mut usage = Usage::default();
        let mut response_model: Option<String> = None;

        while let Some(ev) = sse.next().await {
            if let Some(c) = &cancel
                && c.is_cancelled()
            {
                yield Err(Error::Cancelled);
                return;
            }

            let ev = match ev {
                Ok(e) => e,
                Err(e) => {
                    record_request_failure(&mut request_log, &format!("sse: {e}"), None);
                    yield Err(Error::InvalidResponse(format!("sse: {e}")));
                    return;
                }
            };
            if ev.data == "[DONE]" {
                break;
            }
            if ev.data.is_empty() {
                continue;
            }

            let chunk: Chunk = match serde_json::from_str(&ev.data) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Some(model) = chunk.model {
                response_model = Some(model);
            }
            if let Some(u) = chunk.usage {
                usage.input = u.prompt_tokens;
                usage.output = u.completion_tokens;
                usage.total_tokens = u.total_tokens;
                let cached_from_details = u
                    .prompt_tokens_details
                    .as_ref()
                    .or(u.input_tokens_details.as_ref())
                    .map(|details| details.cached_tokens)
                    .unwrap_or(0);
                usage.cache_read = cached_from_details.max(u.cache_read_input_tokens);
            }

            for choice in chunk.choices {
                if let Some(reason) = choice.finish_reason {
                    stop = match reason.as_str() {
                        "tool_calls" => StopReason::ToolUse,
                        "length" => StopReason::Length,
                        _ => StopReason::Stop,
                    };
                }
                if let Some(delta) = choice.delta {
                    if let Some(thinking) = delta.reasoning_content.or(delta.thinking)
                        && !thinking.is_empty()
                    {
                        if !thinking_started {
                            thinking_started = true;
                            yield Ok(AssistantMessageEvent::ThinkingStart {
                                content_index: next_content_index,
                            });
                        }
                        thinking_buf.push_str(&thinking);
                        yield Ok(AssistantMessageEvent::ThinkingDelta {
                            content_index: next_content_index,
                            delta: thinking,
                        });
                    }
                    if let Some(content) = delta.content
                        && !content.is_empty()
                    {
                        if !text_started {
                            if thinking_started {
                                yield Ok(AssistantMessageEvent::ThinkingEnd {
                                    content_index: next_content_index,
                                    content: thinking_buf.clone(),
                                });
                                next_content_index += 1;
                                thinking_started = false;
                            }
                            text_started = true;
                            yield Ok(AssistantMessageEvent::TextStart {
                                content_index: next_content_index,
                            });
                        }
                        text_buf.push_str(&content);
                        yield Ok(AssistantMessageEvent::TextDelta {
                            content_index: next_content_index,
                            delta: content,
                        });
                    }
                    for tc in delta.tool_calls {
                        let entry = tool_calls.entry(tc.index).or_default();
                        if let Some(id) = tc.id {
                            entry.id = id;
                        }
                        if let Some(function) = tc.function {
                            if let Some(name) = function.name {
                                entry.name = name;
                            }
                            if let Some(arguments) = function.arguments {
                                entry.args.push_str(&arguments);
                                if !tool_started.contains(&tc.index) {
                                    tool_started.insert(tc.index);
                                    yield Ok(AssistantMessageEvent::ToolCallStart {
                                        content_index: next_content_index + tool_started.len(),
                                        id: entry.id.clone(),
                                        name: entry.name.clone(),
                                    });
                                }
                                yield Ok(AssistantMessageEvent::ToolCallDelta {
                                    content_index: next_content_index + tc.index,
                                    delta: arguments,
                                });
                            }
                        }
                    }
                }
            }
        }

        if thinking_started {
            yield Ok(AssistantMessageEvent::ThinkingEnd {
                content_index: next_content_index,
                content: thinking_buf.clone(),
            });
            next_content_index += 1;
        }
        if text_started {
            yield Ok(AssistantMessageEvent::TextEnd {
                content_index: next_content_index,
                content: text_buf.clone(),
            });
            next_content_index += 1;
        }

        let mut content = Vec::new();
        if !thinking_buf.is_empty() {
            content.push(Content::Thinking {
                thinking: thinking_buf,
                thinking_signature: None,
            });
        }
        if !text_buf.is_empty() {
            content.push(Content::Text { text: text_buf });
        }
        for (idx, tc) in tool_calls {
            let args: Value = if tc.args.is_empty() {
                Value::Object(Default::default())
            } else {
                serde_json::from_str(&tc.args).unwrap_or(Value::Object(Default::default()))
            };
            yield Ok(AssistantMessageEvent::ToolCallEnd {
                content_index: next_content_index + idx,
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments: args.clone(),
            });
            content.push(Content::ToolCall {
                id: tc.id,
                name: tc.name,
                arguments: args,
            });
        }

        let message = AssistantMessage {
            content,
            api,
            provider,
            model: response_model.unwrap_or(model_id),
            usage,
            stop_reason: stop,
            error_message: None,
            timestamp: pi_ai::now_ms(),
        };
        if let Some(log) = &mut request_log
            && let Err(error) = log.success(None)
        {
            yield Err(Error::Other(format!("record completed text request: {error:#}")));
            return;
        }
        yield Ok(AssistantMessageEvent::Done {
            reason: stop,
            message,
        });
    };

    Ok(event_stream.boxed())
}

async fn send_chat_request_with_retries(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    headers: &BTreeMap<String, String>,
    body: &Value,
    request_log: &mut Option<crate::request_log::RequestLog>,
) -> pi_ai::Result<Response> {
    let mut last_retryable_error: Option<Error> = None;
    for attempt in 1..=CHAT_REQUEST_MAX_ATTEMPTS {
        if let Some(log) = request_log {
            log.attempt()
                .map_err(|err| Error::Other(format!("record text request attempt: {err:#}")))?;
        }
        let mut req = client
            .post(url)
            .bearer_auth(api_key)
            .header("accept", "text/event-stream")
            .header("content-type", "application/json");
        for (name, value) in headers {
            req = req.header(name, value);
        }

        match req.json(body).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return Ok(resp);
                }
                let body = resp.text().await.unwrap_or_default();
                let err = Error::ProviderError { status: status.as_u16(), body };
                if !is_retryable_http_status(status) || attempt == CHAT_REQUEST_MAX_ATTEMPTS {
                    return Err(err);
                }
                log::debug!("chat request retry {attempt}/{CHAT_REQUEST_MAX_ATTEMPTS} after status {status}");
                last_retryable_error = Some(err);
            }
            Err(err) => {
                let err: Error = err.into();
                if attempt == CHAT_REQUEST_MAX_ATTEMPTS {
                    return Err(err);
                }
                log::debug!("chat request retry {attempt}/{CHAT_REQUEST_MAX_ATTEMPTS} after error: {err}");
                last_retryable_error = Some(err);
            }
        }
        tokio::time::sleep(backoff_delay(attempt)).await;
    }

    Err(last_retryable_error.unwrap_or_else(|| Error::InvalidResponse("chat request retry exhausted".into())))
}

fn record_request_failure(log: &mut Option<crate::request_log::RequestLog>, error: &str, status: Option<u16>) {
    if let Some(log) = log
        && let Err(record_error) = log.failure(error, status)
    {
        log::warn!("record failed text request: {record_error:#}");
    }
}

pub fn build_chat_body(model: &Model, context: &Context, options: &AgnesStreamOptions) -> Value {
    let mut body = json!({
        "model": model.id,
        "messages": convert_messages(context.system_prompt.as_deref(), &context.messages),
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    if let Some(temperature) = options.stream.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(max_tokens) = options.stream.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if options.enable_thinking {
        body["chat_template_kwargs"] = json!({"enable_thinking": true});
    }
    if !context.tools.is_empty() {
        let tools: Vec<Value> = context
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            })
            .collect();
        body["tools"] = json!(tools);
    }
    if options.request_body.is_object() {
        crate::config::deep_merge(&mut body, &options.request_body);
    }
    body
}

fn convert_messages(system_prompt: Option<&str>, messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(system) = system_prompt {
        out.push(json!({"role": "system", "content": system}));
    }
    for message in messages {
        match message {
            Message::User { content, .. } => {
                out.push(json!({"role": "user", "content": user_content(content)}));
            }
            Message::Assistant(assistant) => {
                let mut text = String::new();
                let mut tool_calls = Vec::new();
                for content in &assistant.content {
                    match content {
                        Content::Text { text: value } => text.push_str(value),
                        Content::ToolCall { id, name, arguments } => {
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": arguments.to_string(),
                                }
                            }));
                        }
                        _ => {}
                    }
                }
                let mut msg = json!({"role": "assistant", "content": text});
                if !tool_calls.is_empty() {
                    msg["tool_calls"] = json!(tool_calls);
                }
                out.push(msg);
            }
            Message::ToolResult(result) => {
                out.push(json!({
                    "role": "tool",
                    "tool_call_id": result.tool_call_id,
                    "content": text_content(&result.content),
                }));
            }
        }
    }
    out
}

fn user_content(content: &[Content]) -> Value {
    if !content.iter().any(|part| matches!(part, Content::Image { .. })) {
        return json!(text_content(content));
    }
    let parts: Vec<Value> = content
        .iter()
        .filter_map(|part| match part {
            Content::Text { text } => Some(json!({"type": "text", "text": text})),
            Content::Image { data, mime_type } => {
                let url = if data.starts_with("https://") || data.starts_with("http://") || data.starts_with("data:") {
                    data.clone()
                } else {
                    format!("data:{mime_type};base64,{data}")
                };
                Some(json!({"type": "image_url", "image_url": {"url": url}}))
            }
            _ => None,
        })
        .collect();
    json!(parts)
}

fn text_content(content: &[Content]) -> String {
    content.iter().filter_map(Content::as_text).collect::<Vec<_>>().join("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_ai::{Context, Message, Model, StreamOptions, Tool};
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::thread;
    use std::time::Duration;

    #[test]
    fn chat_body_includes_thinking_flag_and_output_limit() {
        let model = Model::openai_compat(
            "agnes",
            "agnes-3.0-flash",
            "https://apihub.agnes-ai.com/v1",
            524_288,
            65_536,
        );
        let ctx = Context {
            system_prompt: Some("system".into()),
            messages: vec![Message::user_text("hello")],
            tools: vec![],
        };
        let options = AgnesStreamOptions {
            stream: StreamOptions { max_tokens: Some(65_536), ..StreamOptions::default() },
            enable_thinking: true,
            ..AgnesStreamOptions::default()
        };

        let body = build_chat_body(&model, &ctx, &options);

        assert_eq!(body["model"], "agnes-3.0-flash");
        assert_eq!(body["max_tokens"], 65_536);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], true);
    }

    #[test]
    fn chat_body_uses_openai_tool_shape() {
        let model = Model::openai_compat("agnes", "agnes-3.0-flash", "https://example.com/v1", 1, 1);
        let ctx = Context {
            system_prompt: None,
            messages: vec![Message::user_text("use a tool")],
            tools: vec![Tool {
                name: "agnes_task_list".into(),
                description: "List tasks".into(),
                parameters: json!({"type": "object", "properties": {}}),
            }],
        };
        let options = AgnesStreamOptions::default();

        let body = build_chat_body(&model, &ctx, &options);

        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "agnes_task_list");
    }

    #[test]
    fn empty_request_defaults_preserve_the_existing_chat_wire_body() {
        use crate::agent::chat::{ChatOverrides, ChatRuntimeConfig};
        for thinking in [false, true] {
            let cfg = crate::config::AppConfig { chat_thinking: thinking, ..crate::config::AppConfig::default() };
            let runtime = ChatRuntimeConfig::from_app_config(&cfg, ChatOverrides::default()).unwrap();
            let legacy_options = AgnesStreamOptions {
                stream: runtime.stream.clone(),
                enable_thinking: runtime.enable_thinking,
                ..AgnesStreamOptions::default()
            };
            let ctx = Context { system_prompt: None, messages: vec![Message::user_text("hello")], tools: vec![] };
            assert_eq!(
                build_chat_body(&runtime.model, &ctx, &runtime.stream_options()),
                build_chat_body(&runtime.model, &ctx, &legacy_options)
            );
        }
    }

    #[test]
    fn vision_content_preserves_text_image_order_and_input_forms() {
        let content = vec![
            Content::text("Compare these images"),
            Content::Image { data: "https://example.com/photo.png".into(), mime_type: "image/*".into() },
            Content::text("with this local image"),
            Content::Image { data: "aW1hZ2U=".into(), mime_type: "image/jpeg".into() },
            Content::Image { data: "data:image/png;base64,cG5n".into(), mime_type: "image/png".into() },
        ];
        assert_eq!(
            user_content(&content),
            json!([
                {"type": "text", "text": "Compare these images"},
                {"type": "image_url", "image_url": {"url": "https://example.com/photo.png"}},
                {"type": "text", "text": "with this local image"},
                {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,aW1hZ2U="}},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,cG5n"}}
            ])
        );
        assert_eq!(
            user_content(&[Content::text("plain "), Content::text("text")]),
            json!("plain text")
        );
    }

    #[tokio::test]
    async fn stream_sends_vision_history_and_text_followup_to_chat_completions() {
        let server = RetryServer::spawn(vec![200]);
        let model = Model::openai_compat("agnes", "agnes-3.0-flash", server.base_url(), 1, 1);
        let ctx = Context {
            system_prompt: Some("Analyze the supplied pictures".into()),
            messages: vec![
                Message::User {
                    content: vec![
                        Content::text("Describe these"),
                        Content::Image { data: "https://example.com/photo.png".into(), mime_type: "image/*".into() },
                        Content::Image { data: "aW1hZ2U=".into(), mime_type: "image/jpeg".into() },
                    ],
                    timestamp: 0,
                },
                Message::user_text("Which one is brighter?"),
            ],
            tools: vec![],
        };
        let options = AgnesStreamOptions {
            stream: StreamOptions {
                api_key: Some("test-key".into()),
                base_url: Some(server.base_url()),
                ..StreamOptions::default()
            },
            enable_thinking: false,
            ..AgnesStreamOptions::default()
        };
        let mut stream = stream_agnes(&model, &ctx, &options).await.unwrap();
        while let Some(event) = stream.next().await {
            if matches!(event.unwrap(), AssistantMessageEvent::Done { .. }) {
                break;
            }
        }

        let bodies = server.requests();
        assert_eq!(bodies.len(), 1);
        assert_eq!(
            bodies[0]["messages"][1]["content"][1]["image_url"]["url"],
            "https://example.com/photo.png"
        );
        assert_eq!(
            bodies[0]["messages"][1]["content"][2]["image_url"]["url"],
            "data:image/jpeg;base64,aW1hZ2U="
        );
        assert_eq!(bodies[0]["messages"][2]["content"], "Which one is brighter?");
    }

    #[tokio::test]
    async fn stream_keeps_configured_siblings_and_unknown_fields_across_explicit_overrides_and_retries() {
        use crate::agent::chat::{ChatOverrides, ChatRuntimeConfig};
        let server = RetryServer::spawn(vec![503, 200]);
        let mut cfg = crate::config::AppConfig {
            base_url: server.base_url(),
            request_params: serde_json::from_value(json!({
                "text": {
                    "body": {
                        "temperature": 0.4,
                        "top_p": 0.9,
                        "max_tokens": 1000,
                        "chat_template_kwargs": {"enable_thinking": true, "vendor_mode": "keep"},
                        "vendor_option": {"enabled": true},
                        "nullable_option": null
                    },
                    "models": {"agnes-3.0-flash": {"body": {"chat_template_kwargs": {"model_option": "retain"}}}}
                }
            }))
            .unwrap(),
            ..crate::config::AppConfig::default()
        };
        let runtime = ChatRuntimeConfig::from_app_config(
            &cfg,
            ChatOverrides { thinking: Some(false), max_output_tokens: Some(2048), ..ChatOverrides::default() },
        )
        .unwrap()
        .with_api_key("test-key".into());
        // Configuration changes after starting a run must not affect its retries.
        cfg.request_params.text.body["vendor_option"]["enabled"] = json!(false);
        let mut options = runtime.stream_options();
        options.record_requests = false;
        let ctx = Context { system_prompt: None, messages: vec![Message::user_text("hello")], tools: vec![] };
        let mut stream = stream_agnes(&runtime.model, &ctx, &options).await.unwrap();
        while let Some(event) = stream.next().await {
            if matches!(event.unwrap(), AssistantMessageEvent::Done { .. }) {
                break;
            }
        }
        let bodies = server.requests();
        assert_eq!(bodies.len(), 2);
        assert_eq!(
            bodies[0], bodies[1],
            "retry must send the same resolved parameter snapshot"
        );
        let body = &bodies[0];
        assert_eq!(body["max_tokens"], 2048);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(body["chat_template_kwargs"]["vendor_mode"], "keep");
        assert_eq!(body["chat_template_kwargs"]["model_option"], "retain");
        assert_eq!(body["temperature"], 0.4);
        assert_eq!(body["top_p"], 0.9);
        assert_eq!(body["vendor_option"]["enabled"], true);
        assert!(body.get("nullable_option").is_some_and(Value::is_null));
        assert_eq!(body["messages"][0]["content"], "hello");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[tokio::test]
    async fn stream_uses_versioned_chat_endpoint_for_gateway_and_v1_bases() {
        for suffix in ["", "/", "/v1", "/v1/"] {
            for override_base in [false, true] {
                let server = RetryServer::spawn(vec![200]);
                let base_url = format!("{}{suffix}", server.base_url());
                let model = Model::openai_compat("agnes", "agnes-3.0-flash", &base_url, 1, 1);
                let options = AgnesStreamOptions {
                    stream: StreamOptions {
                        api_key: Some("test-key".into()),
                        base_url: override_base.then_some(base_url),
                        ..StreamOptions::default()
                    },
                    enable_thinking: false,
                    ..AgnesStreamOptions::default()
                };
                let ctx = Context { system_prompt: None, messages: vec![Message::user_text("hello")], tools: vec![] };
                let mut stream = stream_agnes(&model, &ctx, &options).await.unwrap();
                while let Some(event) = stream.next().await {
                    if matches!(event.unwrap(), AssistantMessageEvent::Done { .. }) {
                        break;
                    }
                }
                assert_eq!(
                    server.paths(),
                    ["/v1/chat/completions"],
                    "suffix={suffix}, override={override_base}"
                );
                assert_eq!(server.requests()[0]["messages"][0]["content"], "hello");
            }
        }
    }

    #[tokio::test]
    async fn stream_retries_retryable_statuses_before_success() {
        let server = RetryServer::spawn(vec![503, 503, 200]);
        let model = Model::openai_compat("agnes", "agnes-3.0-flash", server.base_url(), 1, 1);
        let ctx = Context { system_prompt: None, messages: vec![Message::user_text("hello")], tools: vec![] };
        let options = AgnesStreamOptions {
            stream: StreamOptions {
                api_key: Some("test-key".into()),
                base_url: Some(server.base_url()),
                ..StreamOptions::default()
            },
            enable_thinking: false,
            ..AgnesStreamOptions::default()
        };

        let mut stream = stream_agnes(&model, &ctx, &options).await.unwrap();
        let mut text = String::new();
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                AssistantMessageEvent::TextDelta { delta, .. } => text.push_str(&delta),
                AssistantMessageEvent::Done { .. } => break,
                _ => {}
            }
        }

        assert_eq!(text, "ok");
        assert_eq!(server.attempts(), 3);
    }

    #[tokio::test]
    async fn stream_errors_after_three_failed_attempts() {
        let server = RetryServer::spawn(vec![503, 503, 503]);
        let model = Model::openai_compat("agnes", "agnes-3.0-flash", server.base_url(), 1, 1);
        let ctx = Context { system_prompt: None, messages: vec![Message::user_text("hello")], tools: vec![] };
        let options = AgnesStreamOptions {
            stream: StreamOptions {
                api_key: Some("test-key".into()),
                base_url: Some(server.base_url()),
                ..StreamOptions::default()
            },
            enable_thinking: false,
            ..AgnesStreamOptions::default()
        };

        let err = match stream_agnes(&model, &ctx, &options).await {
            Ok(_) => panic!("expected provider error after three failed attempts"),
            Err(err) => err,
        };

        assert!(matches!(err, Error::ProviderError { status: 503, .. }));
        assert_eq!(server.attempts(), 3);
    }

    fn read_request_body(stream: &mut std::net::TcpStream) -> (String, Value) {
        let mut request = Vec::new();
        let mut buf = [0; 2048];
        loop {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0, "incomplete HTTP request");
            request.extend_from_slice(&buf[..n]);
            if let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = std::str::from_utf8(&request[..header_end]).unwrap();
                let length = headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                    .unwrap();
                let body_start = header_end + 4;
                if request.len() >= body_start + length {
                    let path = headers
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .to_string();
                    return (
                        path,
                        serde_json::from_slice(&request[body_start..body_start + length]).unwrap(),
                    );
                }
            }
        }
    }

    struct RetryServer {
        addr: std::net::SocketAddr,
        attempts: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<Value>>>,
        paths: Arc<Mutex<Vec<String>>>,
    }

    impl RetryServer {
        fn spawn(statuses: Vec<u16>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let attempts = Arc::new(AtomicUsize::new(0));
            let attempts_for_thread = attempts.clone();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let requests_for_thread = requests.clone();
            let paths = Arc::new(Mutex::new(Vec::new()));
            let paths_for_thread = paths.clone();
            thread::spawn(move || {
                for status in statuses {
                    let (mut stream, _) = listener.accept().unwrap();
                    attempts_for_thread.fetch_add(1, Ordering::SeqCst);
                    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                    let (path, body) = read_request_body(&mut stream);
                    paths_for_thread.lock().unwrap().push(path);
                    requests_for_thread.lock().unwrap().push(body);
                    if status == 200 {
                        let body = concat!(
                            "data: {\"model\":\"agnes-3.0-flash\",\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
                            "data: {\"choices\":[{\"finish_reason\":\"stop\",\"delta\":{}}]}\n\n",
                            "data: [DONE]\n\n",
                        );
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}"
                        )
                        .unwrap();
                    } else {
                        write!(
                            stream,
                            "HTTP/1.1 {status} Service Unavailable\r\nContent-Length: 10\r\nConnection: close\r\n\r\ntry again\n"
                        )
                        .unwrap();
                    }
                }
            });
            Self { addr, attempts, requests, paths }
        }

        fn requests(&self) -> Vec<Value> {
            self.requests.lock().unwrap().clone()
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }

        fn paths(&self) -> Vec<String> {
            self.paths.lock().unwrap().clone()
        }

        fn attempts(&self) -> usize {
            self.attempts.load(Ordering::SeqCst)
        }
    }
}
