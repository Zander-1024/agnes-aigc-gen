---
name: agnes-aigc-gen
description: >-
  Generates images and videos through the agnes-aigc-gen CLI (Agnes AI API).
  Use when calling image/video generation, batch images, image-to-video with asset://,
  or parsing CLI JSON output. For installing the tool, skill, or API key setup, read
  SETUP.md in the same directory — not this file.
---

# agnes-aigc-gen

CLI for [Agnes AI](https://agnes-ai.com) image and video generation.

> **Install & config:** see [SETUP.md](./SETUP.md) in this directory.
> **API reference:** see `docs/agnes-image-2.1-flash.md`, `docs/agnes-video-v2.0.md` in the repo root.

## Quick checklist (before every call)

| Rule | Detail |
|------|--------|
| **Never pass `--size`** | Use `--ratio` only |
| **Image ratios** | Only `1:1`, `4:3`, `3:4`, `16:9`, `9:16` |
| **Image inputs** | Local path, HTTPS URL, `asset://`, base64, **data URI** (i2i) |
| **Video inputs** | **HTTPS URL or `asset://` only** — no local path, base64, or data URI |
| **Video images** | `-i` / `--image` only (not `--input`, `--first-frame`, `--keyframes`) |
| **Batch `-n`** | Image only, **1–4**, concurrent; **mutually exclusive with `--seed`** |
| **Image seed** | `-s` / `--seed`, **0–999** or omit (random per call) |
| **Video seed** | `-s` / `--seed`, **0–999** or omit (not sent to API) |
| **Default `uri`** | Remote HTTPS URL; `--save` for local file |
| **Video max duration** | `floor(441 / frame_rate)`; 24 fps → max **18s** |
| **Video `frame_rate`** | **1–60** (default 24) |
| **Video prompt** | Required unless `--task-id` |
| **Chain i2v** | Generate image first → use `asset_uri` as `-i asset://<id>` |
| **Verbose** | Global `-v` / `--verbose` only |
| **Downloads** | `--save` on command; not `config save-local` |
| **`num_inference_steps`** | Not supported by CLI; never sent for image or video |
| **Chat video tools** | Prefer async video submission; use task tools to follow progress |

**Preconditions:** CLI on PATH, API key configured (see SETUP.md).

## Flag conventions

Every option has a **long form** (`--prompt`, `--ratio`, …). Short forms (`-p`, `-r`, …) are optional shortcuts where defined.

| Command | Short | Long |
|---------|-------|------|
| image | `-p` | `--prompt` |
| image | `-r` | `--ratio` |
| image | `-n` | `--count` |
| image | `-s` | `--seed` |
| image | `-i` | `--input` |
| video | `-p` | `--prompt` |
| video | `--np` | `--negative-prompt` |
| video | `-s` | `--seed` |
| video | `-r` | `--ratio` |
| video | `-d` | `--duration` |
| video | `-f` | `--frame-rate` |
| video | `-i` | `--image` |

## Global flags

```bash
agnes-aigc-gen -v image ...
agnes-aigc-gen --verbose video ...
```

Without `-v`, only JSON/plain result on stdout.

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
| `POST {base_url}/images/generations` | Image |
| `POST {base_url}/videos` + poll `GET https://apihub.agnes-ai.com/agnesapi?video_id=…` | Video |

Default `base_url`: `https://apihub.agnes-ai.com/v1`.

### Before calling

1. API key configured (`config show`)
2. Image: local `-i` paths exist if used
3. Video: every `-i` resolves to an HTTPS URL (`asset://` or direct URL)
4. `asset://` ids from prior runs on **this** machine
5. Video: allow minutes for poll; use `-v` to monitor

---

## Image generation

```bash
agnes-aigc-gen image -p "A cat on the beach" --ratio 16:9
agnes-aigc-gen image -p "Make it cyberpunk" --ratio 9:16 -i ./photo.png
agnes-aigc-gen image -p "portrait" --ratio 9:16 -n 4
agnes-aigc-gen image -p "fixed look" --ratio 1:1 -s 42
```

### Ratios & sizes

| Ratio | Size |
|-------|------|
| `1:1` | 1024×1024 |
| `4:3` | 1152×864 |
| `3:4` | 864×1152 |
| `16:9` | 1280×720 |
| `9:16` | 720×1280 |

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
| `-n` / `--count` | `1` | Max 4; exclusive with `--seed` |
| `-s` / `--seed` | random | 0–999; sent in `extra_body.seed` |
| `-i` / `--input` | — | Repeatable; URL, path, data URI, `asset://` |
| `--save` | off | Local download |
| `--output-format` | `json` | or `plain` |

---

## Video generation

```bash
agnes-aigc-gen video -p "Ocean sunset" --ratio 16:9 -d 5
agnes-aigc-gen video -p "Gentle motion" --ratio 9:16 -d 3 \
  --negative-prompt "blurry, low quality, watermark" \
  -i asset://c8d4eb63a84b
agnes-aigc-gen video -p "Repeatable motion" -s 100 -d 5
agnes-aigc-gen video --task-id video_xxxxxxxx
agnes-aigc-gen video -p "Ocean sunset" --async   # returns local id + vendor query id
agnes-aigc-gen task list                         # refreshes in-progress; shows ID column
agnes-aigc-gen task list --output-format json     # agent-friendly task records
agnes-aigc-gen task show 3                        # local id (or vendor query id)
agnes-aigc-gen task wait 3                         # poll until done
```

**Video query ids:** Agnes now prefers `video_id` for result queries via `/agnesapi?video_id=...`. The legacy `task_id` path remains `/v1/videos/{task_id}`. The CLI stores the preferred vendor query id plus a local **`id`** (1, 2, 3…) in SQLite — use `task show 3` / `task wait 3` instead of the long vendor id.

### Input modes

| Images | API shape |
|--------|-----------|
| 0 | Text-to-video |
| 1 | Image-to-video (`image` URL) |
| 2+ | Multi-image (`extra_body.image` URLs; no `mode` set) |

All frame URLs must share the same aspect ratio. CLI does **not** upload local files or call the image API to stage frames.

### Duration limits

`max_duration = floor(441 / frame_rate)` — e.g. **18s @ 24fps**. `num_frames` snapped to **8n+1** (≤ 441).

### Polling

| Elapsed | Interval |
|---------|----------|
| ≤ 2 min | 30s |
| > 2 min | 15s |

Wait for command to finish; queued tasks are normal. Use `--async` to submit without waiting; track with `task list` / `task show <id>` / `task wait <id>`.

### Video async tasks

| Command | Purpose |
|---------|---------|
| `video --async` | Submit task; JSON includes local `id` + vendor query id as `task_id` |
| `task list [-n N]` | Last N tasks (refreshes **processing** from API); table columns: ID, QUERY ID, PHASE, STATUS, PROGRESS, PROMPT, URI, followed by full Result URLs |
| `task list --output-format json` | Agent-friendly JSON array, including `progress` |
| `task show <ref>` | `<ref>` = local id (`3`, `#3`) or vendor query id |
| `task wait <ref>` | Same as `video --task-id` (block until complete) |

Vendor query id prefers Agnes `video_id` and uses the `/agnesapi` query endpoint. Legacy `id`/`task_id` responses are still accepted through `/v1/videos/{task_id}`. Local `id` is assigned by SQLite on submit.

### Video flags

| Flag | Default | Notes |
|------|---------|-------|
| `-p` / `--prompt` | required* | *Optional with `--task-id` |
| `--np` / `--negative-prompt` | — | Top-level API field |
| `-s` / `--seed` | omit | 0–999; top-level; only sent when set |
| `-r` / `--ratio` | `16:9` | Used for t2v sizing |
| `-d` / `--duration` | `5` | ≤ max duration |
| `-f` / `--frame-rate` | `24` | **1–60** |
| `-i` / `--image` | — | HTTPS URL or `asset://` only |
| `--async` | off | Submit without polling |
| `--task-id` | — | Poll existing task (sync) |
| `--save` | off | |

---

## Workflow: image → video

```bash
agnes-aigc-gen image -p "Portrait, soft light" --ratio 9:16
# → parse asset_uri from JSON

agnes-aigc-gen -v video -p "Subtle motion" --ratio 9:16 -d 3 -i asset://<id>
```

Use **`asset_uri`** (remote URL in DB), not a local path from `--save`. Match ratio. `-d` ≤ 18 @ 24fps.

---

## Assets & history

```bash
agnes-aigc-gen asset list
agnes-aigc-gen asset show asset://c8d4eb63a84b
agnes-aigc-gen history list
```

---

## Agent chat

```bash
agnes-aigc-gen chat
agnes-aigc-gen chat --prompt "Create image concepts, generate one, then submit i2v"
agnes-aigc-gen chat --auto
agnes-aigc-gen chat --no-thinking --context-tokens 128k --max-output-tokens 16k
agnes-aigc-gen chat --resume <session-id>
```

Chat is a PI-based terminal agent with:

- PI coding tools: `read`, `write`, `edit`, `bash`, `web_fetch`, `todo`, search/list tools
- Agnes tools: `agnes_generate_image`, `agnes_submit_video`, `agnes_task_list/show/wait`, `agnes_asset_list/show`, `agnes_history_list/show`, `load_skill`
- Slash commands: `/help`, `/new`, `/sessions`, `/resume <id>`, `/skills`, `/skill <name>`, `/tools`, `/approval`, `/compress`, `/model`, `/thinking`, `/tasks`, `/retry`, `/quit`
- TUI keys: `Tab` completes slash commands, `Shift-Tab` or `/approval toggle` switches approval mode (review ↔ auto; dangerous commands still reviewed in auto), `Ctrl-R` retries the previous request without adding another user message to session history, `Ctrl-T` toggles thinking details, `Ctrl-E` toggles tool-call details, `Ctrl-Q` quits
- Approval panel: `←/→` selects Approve / Allow session / Deny, `Enter` confirms, `↑/↓` or `PgUp/PgDn` scrolls arguments; `y`/`a`/`n` shortcuts still work
- Context: header shows context usage % and cache hit; auto-compresses session history at ≥90% (see [`docs/chat-context-compression.md`](../../docs/chat-context-compression.md)); `/compress` forces compression
- Thinking and tool details are folded by default; expand them when auditing agent behavior
- Chat completion requests automatically try up to 3 total attempts for retryable network failures, `408`, `429`, and `5xx` responses before surfacing an error

Model defaults:

| Setting | Default |
|---------|---------|
| `chat_thinking` | `true` |
| `chat_context_tokens` | `262144` (`256k`) |
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
| Chat | `agnes-2.0-flash` | `POST /v1/chat/completions` |
| Image | `agnes-image-2.1-flash` | `POST /v1/images/generations` |
| Video | `agnes-video-v2.0` | `POST /v1/videos` + poll |

---

## Common errors (runtime)

| Error | Fix |
|-------|-----|
| `API key not configured` | SETUP.md → `config set api-key` |
| Decrypt failed | Re-set API key on this machine |
| `invalid ratio` | Use supported ratio |
| `count must be 1–4` | Image `-n` only |
| `--seed ... cannot be used with --count` | Drop `--seed` for batch, or use `-n 1` |
| `frame_rate must be 1–60` | Fix `-f` / `--frame-rate` |
| `duration ... exceeds maximum` | Reduce `-d` |
| `--prompt is required` | Add `-p` |
| `asset not found` | Regenerate image; `asset list` |
| Video: `unsupported input` / local path | Use `asset://` or HTTPS URL only |
| Hung video | Wait; `-v` or `--task-id` |

---

## Invalid flags (do not use)

`--size`, `--no-save`, `--first-frame`, `--keyframes`, `--input` on video, `-n` on video, `remote_url` in output, `config save-local` for downloads, `num_inference_steps`.

---

## Other commands

`dashboard` (ratatui), `chat` (PI-based agent TUI). For non-interactive agents, prefer `image` / `video` with JSON stdout unless the task specifically needs the chat agent.
