# Agnes 3.0 Flash 接入参考

来源：[官方模型文档](https://wiki.agnes-ai.com/zh-Hans/docs/agnes-30-flash)。核对日期：2026-10-31。本文整理本项目接入所需信息；模型权限与实际可用性以 Agnes 平台为准。

## 模型与接口

| 项目 | 值 |
|------|-----|
| 模型 ID | `agnes-3.0-flash` |
| Base URL | `https://apihub.agnes-ai.com/v1` |
| CLI chat 使用的接口 | `POST /v1/chat/completions` |
| 上下文窗口 | 512K（524,288 Token） |
| 最大输出 | 65,536 Token |
| 官方支持的输入 | 文本、公开可访问的图像 URL |
| 输出 | 文本；支持流式输出、工具调用和 Thinking |

官网另提供 `POST /v1/responses` 和 Anthropic 兼容的 `POST /v1/messages`。本项目的 `chat` 使用 Chat Completions；当前对话输入发送文本。

Agnes 3.0 Pro 的模型 ID 为 `agnes-3.0-pro`，[官方页面](https://wiki.agnes-ai.com/zh-Hans/docs/agnes-30-pro)目前标为“即将上线”，本项目默认使用 3.0 Flash。

## Chat Completions 参数

使用 `Authorization: Bearer YOUR_API_KEY` 认证，请求体为 JSON。

| 参数 | 用途 |
|------|------|
| `model` | 必填，`agnes-3.0-flash` |
| `messages` | 必填，对话消息；包含 `system`、`user`、`assistant`，工具结果使用 `tool` |
| `temperature` / `top_p` | 可选，采样控制 |
| `max_tokens` | 可选，输出预算，上限 65,536 |
| `stream` | 可选，`true` 启用流式返回 |
| `tools` / `tool_choice` | 可选，函数定义及调用选择 |
| `chat_template_kwargs.enable_thinking` | 可选，`true` 启用 Thinking |

```bash
curl https://apihub.agnes-ai.com/v1/chat/completions \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "agnes-3.0-flash",
    "messages": [{"role": "user", "content": "为图片生成工具设计一份简短的实现计划。"}],
    "max_tokens": 2048,
    "stream": true,
    "chat_template_kwargs": {"enable_thinking": true}
  }'
```

普通响应从 `choices[].message.content` 读取正文；流式响应逐段读取 `choices[].delta`。模型返回 `tool_calls` 后，应用执行函数，将结果以对应 `tool_call_id` 的 `tool` 消息追加到对话，再请求下一轮。

## CLI 配置

```bash
agnes-aigc-gen config set text-model agnes-3.0-flash
agnes-aigc-gen config set chat-context-tokens 512k
agnes-aigc-gen config set chat-max-output-tokens 64k
agnes-aigc-gen chat
```

已有配置保留原值；若此前单独指定了 Thinking 模型，可执行 `agnes-aigc-gen config set thinking-text-model ""`，让它跟随 `text-model`。用 `chat --no-thinking` 选择普通对话模式。
