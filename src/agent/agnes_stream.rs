use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use async_stream::stream;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use pi_ai::{
    AssistantMessage, AssistantMessageEvent, AssistantMessageEventStream, Content, Context, Error, Message, Model,
    StopReason, StreamOptions, Usage,
};
use reqwest::{Response, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

const CHAT_REQUEST_MAX_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Default)]
pub struct AgnesStreamOptions {
    pub stream: StreamOptions,
    pub enable_thinking: bool,
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
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = build_chat_body(model, context, options);
    let cancel = options.stream.cancel.clone();
    let headers = options.stream.headers.clone();
    let client = reqwest::Client::new();

    let resp = send_chat_request_with_retries(&client, &url, &api_key, &headers, &body).await?;

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
) -> pi_ai::Result<Response> {
    let mut last_retryable_error: Option<Error> = None;
    for attempt in 1..=CHAT_REQUEST_MAX_ATTEMPTS {
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
                if !should_retry_chat_status(status) || attempt == CHAT_REQUEST_MAX_ATTEMPTS {
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
        tokio::time::sleep(chat_retry_delay(attempt)).await;
    }

    Err(last_retryable_error.unwrap_or_else(|| Error::InvalidResponse("chat request retry exhausted".into())))
}

fn should_retry_chat_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::SERVICE_UNAVAILABLE
        || status.is_server_error()
}

fn chat_retry_delay(attempt: u32) -> Duration {
    Duration::from_millis(100 * attempt as u64)
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
                let text = text_content(content);
                out.push(json!({"role": "user", "content": text}));
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
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::thread;
    use std::time::Duration;

    #[test]
    fn chat_body_includes_thinking_flag_and_output_limit() {
        let model = Model::openai_compat(
            "agnes",
            "agnes-2.0-flash",
            "https://apihub.agnes-ai.com/v1",
            262_144,
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
        };

        let body = build_chat_body(&model, &ctx, &options);

        assert_eq!(body["model"], "agnes-2.0-flash");
        assert_eq!(body["max_tokens"], 65_536);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], true);
    }

    #[test]
    fn chat_body_uses_openai_tool_shape() {
        let model = Model::openai_compat("agnes", "agnes-2.0-flash", "https://example.com/v1", 1, 1);
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

    #[tokio::test]
    async fn stream_retries_retryable_statuses_before_success() {
        let server = RetryServer::spawn(vec![503, 503, 200]);
        let model = Model::openai_compat("agnes", "agnes-2.0-flash", server.base_url(), 1, 1);
        let ctx = Context { system_prompt: None, messages: vec![Message::user_text("hello")], tools: vec![] };
        let options = AgnesStreamOptions {
            stream: StreamOptions {
                api_key: Some("test-key".into()),
                base_url: Some(server.base_url()),
                ..StreamOptions::default()
            },
            enable_thinking: false,
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
        let model = Model::openai_compat("agnes", "agnes-2.0-flash", server.base_url(), 1, 1);
        let ctx = Context { system_prompt: None, messages: vec![Message::user_text("hello")], tools: vec![] };
        let options = AgnesStreamOptions {
            stream: StreamOptions {
                api_key: Some("test-key".into()),
                base_url: Some(server.base_url()),
                ..StreamOptions::default()
            },
            enable_thinking: false,
        };

        let err = match stream_agnes(&model, &ctx, &options).await {
            Ok(_) => panic!("expected provider error after three failed attempts"),
            Err(err) => err,
        };

        assert!(matches!(err, Error::ProviderError { status: 503, .. }));
        assert_eq!(server.attempts(), 3);
    }

    struct RetryServer {
        addr: std::net::SocketAddr,
        attempts: Arc<AtomicUsize>,
    }

    impl RetryServer {
        fn spawn(statuses: Vec<u16>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let attempts = Arc::new(AtomicUsize::new(0));
            let attempts_for_thread = attempts.clone();
            thread::spawn(move || {
                for status in statuses {
                    let (mut stream, _) = listener.accept().unwrap();
                    attempts_for_thread.fetch_add(1, Ordering::SeqCst);
                    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                    let mut buf = [0; 2048];
                    let _ = stream.read(&mut buf);
                    if status == 200 {
                        let body = concat!(
                            "data: {\"model\":\"agnes-2.0-flash\",\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
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
            Self { addr, attempts }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }

        fn attempts(&self) -> usize {
            self.attempts.load(Ordering::SeqCst)
        }
    }
}
