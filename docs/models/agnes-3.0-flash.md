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

官网另提供 `POST /v1/responses` 和 Anthropic 兼容的 `POST /v1/messages`。本项目的 `chat` 使用 Chat Completions；支持文本和图片内容一起发送。

Agnes 3.0 Pro 的模型 ID 为 `agnes-3.0-pro`，[官方页面](https://wiki.agnes-ai.com/zh-Hans/docs/agnes-30-pro)目前标为“即将上线”，本项目默认使用 3.0 Flash。

## Chat Completions 参数

使用 `Authorization: Bearer YOUR_API_KEY` 认证，请求体为 JSON。

| 参数 | 用途 |
|------|------|
| `model` | 必填，`agnes-3.0-flash` |
| `messages` | 必填，对话消息；包含 `system`、`user`、`assistant`，工具结果使用 `tool` |
| `messages[].content` | 纯文本字符串，或包含 `text`、`image_url` 的内容块数组 |
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

## 图片理解

公开图片 URL 可直接作为 `image_url.url` 与文本同传：

```json
{
  "model": "agnes-3.0-flash",
  "messages": [{
    "role": "user",
    "content": [
      {"type": "text", "text": "请描述图片并列出可见文字。"},
      {"type": "image_url", "image_url": {"url": "https://example.com/photo.jpg"}}
    ]
  }]
}
```

CLI 同时接受本地图片、裸 Base64、`base64:...` 和完整 Data URI。客户端会验证本地／内联 PNG、JPEG、WebP 图片，保留原始字节与 MIME 类型，将其编码为 Data URI 并放进同一个 `image_url.url` 字段；远程 URL 原样传给文本接口，不经过图片生成接口。

```bash
agnes-aigc-gen chat -p "请描述这张图片" -i https://example.com/photo.jpg
agnes-aigc-gen chat -p "比较两张图片" -i ./first.png -i ./second.jpg
agnes-aigc-gen chat -p "读取图片里的文字" -i "data:image/png;base64,BASE64_HERE"
agnes-aigc-gen chat -p "描述图片" -i "base64:BASE64_HERE"
agnes-aigc-gen chat -i ./photo.png
```

每个 `-i` 参数只表示一张图片，多图请重复 `-i`；URL、文件名与 Data URI 中的逗号均会保留。未指定 `-p` 时进入交互聊天，图片等待下一条消息发送。交互命令：

| 命令 | 用途 |
|------|------|
| `/image <URL／路径／Base64>` | 为下一条消息添加图片；路径可含空格，也可加配对外引号 |
| `/images` | 查看待发送图片 |
| `/images clear` | 清空待发送图片 |

重试和保存／恢复会话会保留已发送图片；新建或切换会话清除未发送附件。自动或手动上下文压缩可能将旧图片替换为 `[image]` 摘要。

验证：2026-10-31 使用本地 PNG 转为 Data URI，向 `agnes-3.0-flash` 发起真实请求，返回 HTTP 200，模型正确识别测试图左右两侧的绿色与洋红色。本地回归测试另覆盖 URL、本地文件、Base64 各种写法、实际请求内容及多轮会话保留。

## CLI 配置

```bash
agnes-aigc-gen config set text-model agnes-3.0-flash
agnes-aigc-gen config set chat-context-tokens 512k
agnes-aigc-gen config set chat-max-output-tokens 64k
agnes-aigc-gen chat
```

已有配置保留原值；若此前单独指定了 Thinking 模型，可执行 `agnes-aigc-gen config set thinking-text-model ""`，让它跟随 `text-model`。用 `chat --no-thinking` 选择普通对话模式。
