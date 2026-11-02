---
name: agnes-aigc-gen
description: >-
  Uses the agnes-aigc-gen CLI for image/video generation and text-model image understanding.
  Use for batch images, image-to-video with asset://, chat image attachments,
  or parsing CLI JSON output. For installing the tool, skill, or API key setup, read
  SETUP.md in the same directory — not this file.
---

# agnes-aigc-gen

CLI for [Agnes AI](https://agnes-ai.com) text chat, image understanding, and image/video generation.

> **Install & config:** see [SETUP.md](./SETUP.md) in this directory.
> **API reference:** in the repository, read `docs/models/agnes-image-2.5-flash.md` for image parameters, `docs/models/agnes-video-2.5.md` and `docs/models/agnes-video-2.5-flash.md` for video modes and limits, or `docs/models/agnes-3.0-flash.md` for text chat. Installed skills can use the official links in **Models & endpoints**.

## Quick checklist (before every call)

| Rule | Detail |
|------|--------|
| **Image ratios** | `1:1`, `3:4`, `4:3`, `16:9`, `9:16`, `2:3`, `3:2`, `21:9` |
| **Image size** | `--size` `1K` / `2K` / `3K` / `4K` (default `1K`; max `4K`; case-insensitive on CLI) |
| **Image inputs** | Local path, HTTPS URL, `asset://`, base64, **data URI** (i2i) |
| **Video inputs** | **HTTPS URL or `asset://` only** — no local path, base64, or data URI |
| **Video media** | `-i` / `--image`, `--first-frame`, `--last-frame`, `--audio`, `--video`; public HTTPS URLs or image `asset://` |
| **Batch `-n`** | Image only, **1–4**, concurrent; **mutually exclusive with `--seed`** |
| **Image seed** | `-s` / `--seed`, **0–999** or omit (random per call) |
| **Video seed** | `-s` / `--seed`, unsigned integer or omit (not sent to API) |
| **Default `uri`** | Remote HTTPS URL; `--save` for local file |
| **Video duration** | Integer **4–12 seconds**, default 5 |
| **Video size** | `720P` / `1080P` / `1K` / `2K`; Flash accepts **720P only** |
| **Video model** | Default `agnes-video-2.5`; `--model agnes-video-2.5-flash` selects Flash |
| **Video prompt** | Required unless `--task-id` |
| **Chain i2v** | Generate image first → use `asset_uri` as `-i asset://<id>` |
| **Version** | Global `-v` / `-V` prints version and exits |
| **Verbose** | Global `--verbose` only (no `-v` short form) |
| **Downloads** | `--save` on command; not `config save-local` |
| **`num_inference_steps`** | Not supported by CLI; never sent for image or video |
| **Chat video tools** | Prefer async video submission; use task tools to follow progress |
| **Chat vision** | `chat -i` / `--image`: URL, local image, bare Base64, `base64:...`, or Data URI; repeat `-i` for multiple images |

**Preconditions:** CLI on PATH, API key configured (see SETUP.md).

## Flag conventions

Every option has a **long form** (`--prompt`, `--ratio`, …). Short forms (`-p`, `-r`, …) are optional shortcuts where defined.

| Command | Short | Long |
|---------|-------|------|
| image | `-p` | `--prompt` |
| image | `-r` | `--ratio` |
| image | | `--size` |
| image | `-n` | `--count` |
| image | `-s` | `--seed` |
| image | `-i` | `--input` |
| video | `-p` | `--prompt` |
| video | `-s` | `--seed` |
| video | `-r` | `--ratio` |
| video | `-d` | `--duration` |
| video | `-i` | `--image` |
| video | | `--model`, `--mode`, `--size` |
| video | | `--first-frame`, `--last-frame`, `--audio`, `--video` |
| chat | `-p` | `--prompt` |
| chat | `-i` | `--image` |

## Global flags

```bash
agnes-aigc-gen -v                  # version
agnes-aigc-gen --verbose image ...
agnes-aigc-gen --verbose video ...
```

Without `--verbose`, only JSON/plain result on stdout.

## Version, update, uninstall

| Command | Purpose |
|---------|---------|
| `-v` / `-V` | Print current version and exit |
| `version` | Same as `-v`; `version check` compares GitHub latest; `version changelog` prints CHANGELOG |
| `self update` | Download newer release binary; interactively offers skill refresh to previous or default paths |
| `self update -y` | Update binary only, no prompts |
| `self update -y --update-skill` | Update binary + skill at recorded `install.toml` targets |
| `self uninstall` | Stepwise remove binary, skills, and optionally config/data |

Install locations are recorded in `{config_dir}/install.toml` after `install.sh` / `install-remote.sh`.

## Runtime file dependencies

Do **not** assume `~/.config` on macOS.

### Config & state (`{config_dir}`)

| OS | `{config_dir}` |
|----|----------------|
| macOS | `~/Library/Application Support/agnes-aigc-gen/` |
| Linux | `~/.config/agnes-aigc-gen/` |
| Windows | `%APPDATA%\agnes-aigc-gen\` |

| File | Purpose |
|------|---------|
| `config.toml` | Settings + encrypted API key (must exist before API calls) |
| `generations.db` | SQLite: `asset://` → remote URL, generation history |
| `chat_sessions/*.json` | Chat TUI transcripts for `/sessions` and `--resume` |

### Agent sandbox & write permissions

Every successful `image`, `video`, `video --async`, and related command **writes to `{config_dir}`** (SQLite `generations.db`, asset records, async video tasks). With `--save`, output also goes to `output_dir`. These paths are **outside the project workspace**.

Some agent runtimes (e.g. Cursor sandbox) only allow writes inside the workspace. If a CLI call fails with permission denied, sandbox, or “cannot write to …” errors pointing at `{config_dir}` or `generations.db`:

1. **Stop and tell the user** — the CLI needs write access to `{config_dir}`; ask them to grant it (disable sandbox, allow “all” permissions, or add `{config_dir}` to the agent’s allowed write paths).
2. **Do not bypass** — do not relocate config/DB into the workspace, fake JSON output, skip the CLI, or use undocumented env overrides to dodge the restriction. Those break `asset://` chaining, task tracking, and history on this machine.

Example user message:

> `agnes-aigc-gen` writes state to `~/Library/Application Support/agnes-aigc-gen/` (macOS) or `~/.config/agnes-aigc-gen/` (Linux). This run was blocked by the agent sandbox. Please allow writes to that directory (or run with full permissions), then retry the same command.

### Image inputs (`-i` / `--input`)

| Form | Image i2i | Video `-i` |
|------|-----------|------------|
| Local path | ✓ | ✗ |
| `https://…` | ✓ | ✓ |
| `asset://<id>` | ✓ (→ remote URL) | ✓ (→ remote URL) |
| base64 / data URI | ✓ | ✗ |

**Video:** the API accepts **HTTP(S) image URLs only**. Do not pass local files or inline base64. Run `image` first, then chain with `asset://<id>` from JSON output (or use a public HTTPS URL).

**Image i2i:** data URIs (`data:image/jpeg;base64,…`) and local files are supported; PNG/local bytes are converted to JPEG data URI before upload.

### Outputs

- Default: **no local files**; `uri` = remote URL
- `--save`: writes `{timestamp}-{hash}.{ext}` under `output_dir` / `--output-dir`

### Network

| Endpoint | When |
|----------|------|
| `POST {gateway}/v1/chat/completions` | Chat |
| `POST {gateway}/v1/images/generations` | Image |
| `POST {gateway}/v1/videos` | Video creation |
| `GET {gateway}/agnesapi?video_id=…&model_name=…` | Video queries, all modes and ID prefixes |

Default `base_url`: `https://apihub.agnes-ai.com`. Existing values ending in `/v1` and trailing slashes are accepted. Here `{gateway}` is the configured base URL with the optional trailing `/v1` and slashes removed; the CLI constructs the endpoint paths above. Queries use the configured gateway, including custom domains.

### Before calling

1. API key configured (`config show`)
2. Image: local `-i` paths exist if used
3. Video: every `-i` resolves to an HTTPS URL (`asset://` or direct URL)
4. `asset://` ids from prior runs on **this** machine
5. Video: allow minutes for poll; use `--verbose` to monitor

---

## Image generation

```bash
agnes-aigc-gen image -p "A cat on the beach" --ratio 16:9 --size 2K
agnes-aigc-gen image -p "Make it cyberpunk" --ratio 9:16 -i ./photo.png
agnes-aigc-gen image -p "portrait" --ratio 9:16 -n 4
agnes-aigc-gen image -p "fixed look" --ratio 1:1 -s 42
```

### Ratios & resolution

| Ratio | Notes |
|-------|-------|
| `1:1` | Square |
| `3:4` | Portrait |
| `4:3` | Landscape |
| `16:9` | Widescreen |
| `9:16` | Vertical video / stories |
| `2:3` | Portrait |
| `3:2` | Landscape |
| `21:9` | Ultrawide |

| `--size` | Meaning |
|----------|---------|
| `1K` | Default resolution |
| `2K` | Higher resolution |
| `3K` | Higher resolution |
| `4K` | Maximum resolution |

CLI accepts `1k` / `1K` interchangeably; API wire value is always uppercase.

### Batch (`-n` / `--count` 2–4)

Concurrent API calls. **Do not combine with `-s` / `--seed`** — batch mode requires omitting seed (each call gets its own random seed).

```json
{
  "results": [
    { "success": true, "type": "image", "uri": "https://...", "asset_uri": "asset://abc" },
    { "success": false, "message": "image generation failed (429): ..." }
  ]
}
```

Single image: top-level object (no `results` wrapper).

### Image flags

| Flag | Default | Notes |
|------|---------|-------|
| `-p` / `--prompt` | required | |
| `-r` / `--ratio` | `1:1` | Supported ratio |
| `--size` | `1K` | Resolution tier (`1K`–`4K`) |
| `-n` / `--count` | `1` | Max 4; exclusive with `--seed` |
| `-s` / `--seed` | random | 0–999; sent in `extra_body.seed` |
| `-i` / `--input` | — | Repeatable; URL, path, data URI, `asset://` |
| `--save` | off | Local download |
| `--output-format` | `json` | or `plain` |

---

## Video generation

```bash
agnes-aigc-gen video -p "Ocean sunset" --ratio 16:9 -d 5 --size 1080P
agnes-aigc-gen video --model agnes-video-2.5-flash -p "Gentle motion" -i asset://c8d4eb63a84b -d 5
agnes-aigc-gen video -p "Smooth transition" --first-frame https://example.com/start.png --last-frame https://example.com/end.png
agnes-aigc-gen video -p "Use <Picture 1> and follow <Audio 1>" --mode reference -i https://example.com/person.png --audio https://example.com/beat.mp3
agnes-aigc-gen video -p "Follow <Video 1>" --video '{"url":"https://example.com/motion.mp4","start_seconds":2,"require_audio":false}'
agnes-aigc-gen video -p "Ocean sunset" --async
agnes-aigc-gen task list --output-format json
agnes-aigc-gen task show 3
agnes-aigc-gen task wait 3
```

### Modes and limits

| Selection | API mode / fields |
|-----------|-------------------|
| No media | `text` |
| One `-i` | `keyframe`, mapped to `first_frame` |
| `--first-frame` and/or `--last-frame` | `keyframe`; at least one |
| Multiple `-i`, or audio/video media | `reference`; `images`, `audios`, `videos` |
| `--mode reference -i URL` | Single-image reference instead of first frame |

`text` takes no media. `keyframe` uses first/last frames, and `reference` uses media arrays; keep these fields within their mode. All media must resolve to publicly accessible HTTPS URLs. Images can use local `asset://` records that resolve to a remote URL. Output ratio is selected by `--ratio`; reference images may have different ratios.

| Limit | Video 2.5 | Video 2.5 Flash |
|-------|-----------|-----------------|
| Duration | Integer 4–12s | Integer 4–12s |
| Size | `720P`, `1080P`, `1K`, `2K` | `720P` |
| Reference images | Up to 8 | Up to 5 |
| Reference audio clips | Up to 3 | Up to 3 |
| Reference video clips | Up to 1 | Unsupported |

Video ratios: `21:9`, `16:9`, `4:3`, `1:1`, `3:4`, `9:16`. `1K` is fixed `1024x1024`; other pixel dimensions depend on model and ratio. The service validates file size, media duration, and encoding limits described in the API reference.

### Video flags

| Flag | Default | Notes |
|------|---------|-------|
| `-p` / `--prompt` | required* | *Optional with `--task-id` |
| `--model` | configured `video_model` | `agnes-video-2.5` or `agnes-video-2.5-flash` |
| `--mode` | inferred | `text`, `keyframe`, `reference` |
| `--size` | `720P` | Flash only supports `720P` |
| `-s` / `--seed` | omit | Unsigned integer; top-level API field |
| `-r` / `--ratio` | `16:9` | Output aspect ratio |
| `-d` / `--duration` | `5` | Integer 4–12 seconds |
| `-i` / `--image` | — | Repeatable/comma-separated HTTPS URLs or `asset://` |
| `--first-frame`, `--last-frame` | — | HTTPS URL or `asset://` |
| `--audio` | — | Repeatable HTTPS audio URL |
| `--video` | — | Repeatable HTTPS URL or JSON object with `url`, optional `start_seconds`, `require_audio` |
| `--async` | off | Submit without polling |
| `--task-id` | — | Poll existing task |
| `--save` | off | Download result |

### Async tasks and polling

Creation returns Agnes `video_id`. The CLI prints it as `task_id` alongside SQLite local `id` (1, 2, 3…). Polling always uses `/agnesapi?video_id=...&model_name=...` on the configured gateway, with the model saved at task creation, regardless of whether the ID starts with `task_` or `video_`. Completion uses top-level `status`, `progress`, and result `url`; the parser also accepts `video_url` and `metadata.url`.

| Command | Purpose |
|---------|---------|
| `video --async` | Submit and return immediately |
| `task list [-n N]` | Recent tasks; interactive TTY shows details and copies the selected field on Enter |
| `task list --output-format json` | Agent-friendly task records |
| `task list --output-format table` | Static task table |
| `task show <ref>` | Local id (`3`, `#3`) or vendor query id |
| `task wait <ref>` | Block until completed or failed |

Use `--async` for generation that can take minutes; then follow the local task id. The client uses a conservative polling interval and retries transient failures.

## Workflow: image → video

```bash
agnes-aigc-gen image -p "Portrait, soft light" --ratio 9:16
# → parse asset_uri from JSON

agnes-aigc-gen --verbose video -p "Subtle motion" --ratio 9:16 -d 5 -i asset://<id>
```

Use **`asset_uri`** (remote URL in DB), not a local path from `--save`. Select the output ratio and a 4–12 second duration.

---

## Assets & history

```bash
agnes-aigc-gen asset list                         # interactive TUI in a terminal
agnes-aigc-gen asset list --output-format json
agnes-aigc-gen asset list --output-format table
agnes-aigc-gen asset show asset://c8d4eb63a84b
agnes-aigc-gen history list
```

| Command | Purpose |
|---------|---------|
| `asset list [-n N]` | Last N assets; interactive TTY: table + **Detail** box (Left/Right cycles ASSET URI/REMOTE URL/ID; bottom shows value, max 200 chars); Enter copies **full** field |
| `asset list --output-format table` | Static bordered table |
| `asset list --output-format json` | Agent-friendly JSON array |
| `asset show <id>` | Full asset record JSON (`asset://id` or bare id) |
| `history list` | Plain-text generation audit trail |

---

## Agent chat

```bash
agnes-aigc-gen chat
agnes-aigc-gen chat --prompt "Create image concepts, generate one, then submit i2v"
agnes-aigc-gen chat -p "Describe this image" -i https://example.com/photo.jpg
agnes-aigc-gen chat -p "Compare these images" -i ./first.png -i ./second.jpg
agnes-aigc-gen chat -p "Read this image" -i "data:image/png;base64,BASE64_HERE"
agnes-aigc-gen chat -i ./photo.png   # TUI: attach to the next user message
agnes-aigc-gen chat --auto
agnes-aigc-gen chat --no-thinking --context-tokens 128k --max-output-tokens 16k
agnes-aigc-gen chat --resume <session-id>
```

Chat is a PI-based terminal agent with:

- PI coding tools: `read`, `write`, `edit`, `bash`, `web_fetch`, `todo`, search/list tools
- Agnes tools: `agnes_generate_image`, `agnes_submit_video`, `agnes_task_list/show/wait`, `agnes_asset_list/show`, `agnes_history_list/show`, `load_skill`
- Slash commands: `/image <input>`, `/images`, `/images clear`, `/help`, `/new`, `/sessions`, `/resume <id>`, `/skills`, `/skill <name>`, `/tools`, `/approval`, `/compress`, `/model`, `/thinking`, `/tasks`, `/retry`, `/quit`
- TUI keys: `Tab` completes slash commands, `Shift-Tab` or `/approval toggle` switches approval mode (review ↔ auto; dangerous commands still reviewed in auto), `Ctrl-R` retries the previous request without adding another user message to session history, `Ctrl-T` toggles thinking details, `Ctrl-E` toggles tool-call details, `Ctrl-Q` quits
- Approval panel: `←/→` selects Approve / Allow session / Deny, `Enter` confirms, `↑/↓` or `PgUp/PgDn` scrolls arguments; `y`/`a`/`n` shortcuts still work
- Context: header shows context usage % and cache hit; auto-compresses session history at ≥90% (see [`docs/chat-context-compression.md`](../../docs/chat-context-compression.md)); `/compress` forces compression
- Thinking and tool details are folded by default; expand them when auditing agent behavior
- Chat completion requests automatically try up to 3 total attempts for retryable network failures, `408`, `429`, and `5xx` responses before surfacing an error

### Chat image attachments

`chat -i` / `--image` accepts a URL, local image path, bare Base64, `base64:...`, or full Data URI. Use one image per `-i` argument and repeat the flag for multiple images. Commas inside a URL, filename, or Data URI are preserved. Local/inline PNG/JPEG/WebP images are validated and encoded into `image_url` content parts, preserving their bytes and MIME type; remote URLs are forwarded directly. Chat image understanding calls the text endpoint and does not call image generation.

In the TUI, attach an image with `/image <input>`; the rest of the line is one input, so paths may contain spaces. Matching outer quotes are accepted. `/images` lists pending images without dumping Base64, and `/images clear` clears them. Pending images attach to the next message and are consumed when that request is built successfully. `/new` and `/resume` clear unsent attachments. Retrying uses the saved request with its images; saved sessions retain image content for later turns. Context compression can replace older image content with an `[image]` summary.

Model defaults:

| Setting | Default |
|---------|---------|
| `text_model` | `agnes-3.0-flash` |
| `chat_thinking` | `true` |
| `chat_context_tokens` | `524288` (`512k`) |
| `chat_max_output_tokens` | `65536` (`64k`) |
| `thinking_text_model` | fallback to `text_model` |

Approval behavior:

- Default mode reviews side-effecting tools and media generation.
- `--auto` auto-approves non-dangerous calls.
- Dangerous commands still require human approval, including force push, retagging, `git reset --hard`, `git clean`, `rm -rf`, `sudo`, `curl | sh`, publish/release commands, and writes outside the workspace.

Video tool calls should use `agnes_submit_video`; it submits asynchronously and returns a local task id instead of blocking the conversation.

---

## Models & endpoints

| Type | Model | Endpoint |
|------|-------|----------|
| Chat | [`agnes-3.0-flash`](https://wiki.agnes-ai.com/zh-Hans/docs/agnes-30-flash) | `POST /v1/chat/completions` |
| Image | [`agnes-image-2.5-flash`](https://wiki.agnes-ai.com/zh-Hans/docs/agnes-image-25-flash) | `POST /v1/images/generations` |
| Video | [`agnes-video-2.5`](https://wiki.agnes-ai.com/zh-Hans/docs/agnes-video-25) | `POST /v1/videos` + poll |
| Video Flash | [`agnes-video-2.5-flash`](https://wiki.agnes-ai.com/zh-Hans/docs/agnes-video-25-flash) | `POST /v1/videos` + poll |

Existing saved configs keep their model names. For an upgrade, follow **Upgrading model settings** in SETUP.md before creating new video tasks. Text defaults to 3.0 Flash (512K context); 3.0 Pro is coming soon.

---

## Common errors (runtime)

| Error | Fix |
|-------|-----|
| Permission denied / sandbox blocked write to `{config_dir}` or `generations.db` | Ask user to grant write access to `{config_dir}` (see **Agent sandbox & write permissions**); do not workaround |
| `API key not configured` | SETUP.md → `config set api-key` |
| Decrypt failed | Re-set API key on this machine |
| `invalid ratio` | Use supported ratio |
| `count must be 1–4` | Image `-n` only |
| `--seed ... cannot be used with --count` | Drop `--seed` for batch, or use `-n 1` |
| Invalid video duration/size/mode | Use integer 4–12s, the model’s supported size, and matching media fields |
| `--prompt is required` | Add `-p` |
| `asset not found` | Regenerate image; `asset list` |
| Video: `unsupported input` / local path | Use `asset://` or HTTPS URL only |
| Hung video | Wait; `--verbose` or `--task-id` |

---

## Invalid flags (do not use)

`--frame-rate` / `-f`, `--negative-prompt` / `--np`, `--no-save`, `--keyframes`, `--input` on video, `-n` on video, `remote_url` in output, `config save-local` for downloads, `num_inference_steps`.

---

## Other commands

### Dashboard (`agnes-aigc-gen dashboard`)

Interactive ratatui UI for image/video generation, tasks, assets, and settings.

| Page | Layout / keys |
|------|------|
| **Home** | ↑↓ / Enter / 1–7 shortcuts; **Video Tasks** shows `(N running)` when applicable; Status bar `[RUN N]` globally |
| **Generate Image** | Left: **Prompt** (multiline). Right: **Parameters** (ratio, size, count, seed, save, output) + **References**. Tab = Text / Params / Media; ↑↓ in Params; ←→ ratio/count; `a`/`A`/`d` media; Ctrl+Enter submit; **Result** panel at bottom when done |
| **Generate Video** | Left: **Prompt** (multiline). Right: **Parameters** (model, size, ratio, duration, seed) + **Active/Recent tasks** (symbols, progress bars, 1s refresh when running) + **References**. `t` Tasks · `r` refresh · `g` jump to running · Async toggle; async stays on page with **Result (running)** panel until complete |
| **Video Tasks** | Colored state icons + progress bars; title shows `(N running)`; auto-refresh 1s when running else 5s; `g` select running row |
| **Assets** | List TUI; `v` send to video form; Enter copy field |
| **Settings** | Edit config; `s` / Enter on Actions = save |
| **Chat** | Exits dashboard → full agent chat TUI |

Params lines show resolved **pixel size** (tier) and video **duration** inline (no side preview panel).

`chat` (PI-based agent TUI). For non-interactive agents, prefer `image` / `video` with JSON stdout unless the task specifically needs the chat agent.
