# Chat context compression

Agnes Chat keeps the full `pi_ai::Message` session history in SQLite-backed chat sessions. Long conversations can approach the configured context window (`chat_context_tokens`, default `256k`). This document describes how the TUI estimates usage, when compression runs, and what gets preserved.

## Token estimation

The UI does not tokenize locally. It uses a conservative heuristic:

```
estimated_tokens = (system_prompt_chars + sum(message_chars) + 8 * message_count) / 4
```

- **System prompt** includes the base agent instructions, project files (`AGENTS.md`, `CLAUDE.md`, `.pi/instructions.md`), and loaded skills.
- **Live usage** from the API (`usage.prompt_tokens`) replaces the estimate after each assistant turn when available.
- **Header display** shows `max(estimated_session_tokens, last_prompt_tokens) / context_limit (percent)`.

## Cache hit display

When the provider returns usage metadata, the header also shows cache hits:

- OpenAI-compatible: `usage.prompt_tokens_details.cached_tokens`
- Anthropic-compatible: `usage.cache_read_input_tokens`

Cache hit rate = `cache_read / prompt_tokens`.

If the API omits cache fields, the header shows `—` until the first completed turn.

## When compression runs

| Trigger | Condition |
|---------|-----------|
| **Automatic (pre-request)** | Before sending a user message, estimated or live context ≥ **90%** of `chat_context_tokens` |
| **Automatic (post-turn)** | After a completed agent run, if `last_prompt_tokens` or estimate ≥ **90%** |
| **Manual** | User runs `/compress` in the TUI |

Compression does **not** call the model. It is a deterministic history rewrite.

## Compression algorithm

Implementation: [`src/agent/context.rs`](../src/agent/context.rs)

1. If history length ≤ `PRESERVE_RECENT_MESSAGES + 1` (default **13**), skip — nothing to compress.
2. Split history into:
   - **Older** — all messages except the last **12**
   - **Recent** — the last **12** messages (kept verbatim)
3. Build one replacement **user** message containing a markdown summary of older messages:
   - Each message: role (`user` / `assistant` / `tool`) + truncated text (240 chars)
   - Tool results: tool name + 120-char snippet
   - Thinking blocks: `[thinking]` prefix + 80-char snippet
4. Replace older messages with that single summary message, followed by the preserved recent tail.
5. Persist the compressed session to disk when compression happens after a run or via `/compress`.

## What is preserved vs dropped

| Preserved | Dropped or truncated |
|-----------|----------------------|
| Last 12 messages (full content) | Older message bodies |
| Tool names in summaries | Full tool JSON/output in older turns |
| User prompt snippets (240 chars) | Older thinking streams (snippet only) |
| Session id, model, thinking flag | Exact token-for-token older context |

## Configuration

| Setting | Default | Notes |
|---------|---------|-------|
| `chat_context_tokens` | `262144` | Context window shown in header; compression threshold derived from this |
| Auto threshold | 90% | Constant `AUTO_COMPRESS_THRESHOLD_PERCENT` in `context.rs` |
| Preserve recent | 12 messages | Constant `PRESERVE_RECENT_MESSAGES` |

## Limitations

- Heuristic token counts can diverge from provider billing tokens; prefer `last_prompt_tokens` after each turn.
- Summary compression may drop details the model still needs; keep important facts in recent turns or re-state them after compression.
- No LLM summarization yet — future work could replace the markdown rollup with a dedicated summarization call.

## Related controls

- **Ctrl-Tab** — toggle approval mode (review ↔ auto) at runtime
- **/approval** — show current approval mode
- **/compress** — force compression regardless of threshold (if enough history exists)
- **/model** — show model and token limits
