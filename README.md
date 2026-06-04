# agnes-aigc-gen

CLI and terminal dashboard for [Agnes AI](https://agnes-ai.com) image and video generation.

## Features

- **Image generation** — text-to-image and image-to-image (`agnes-image-2.1-flash`)
- **Video generation** — text-to-video, image-to-video, multi-image (`agnes-video-v2.0`, async poll)
- **Aspect ratios** — pass `--ratio` only; dimensions computed internally (never `--size`)
- **Batch images** — `-n` / `--count` 1–4 concurrent calls with partial-failure JSON
- **Structured output** — JSON with `ratio`, `size`, `uri`, `asset_uri`; remote URL by default (`--save` to download)
- **Asset history** — SQLite `asset://` references for image → video workflows
- **Agent chat TUI** — PI-based terminal agent with tool calls, skills, approvals, thinking mode, and async video submission
- **Encrypted config** — API key encrypted, machine-bound (see config dir in [SETUP.md](skills/agnes-aigc-gen/SETUP.md))
- **Dashboard** — `agnes-aigc-gen dashboard` (ratatui terminal UI)
- **Agent skill** — Cursor skill under `skills/agnes-aigc-gen/`

## Requirements

- Agnes API key from [platform.agnes-ai.com](https://platform.agnes-ai.com)
- **Release install:** curl/PowerShell (no Rust required)
- **Source install:** Rust 1.85+ (edition 2024)

## Install

### Release install (recommended)

Downloads the latest binary from [GitHub Releases](https://github.com/Zander-1024/agnes-aigc-gen/releases).

**macOS / Linux:**

```bash
curl -fsSL https://raw.githubusercontent.com/Zander-1024/agnes-aigc-gen/master/install-remote.sh | bash
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/Zander-1024/agnes-aigc-gen/master/install-remote.ps1 | iex
```

**Windows (cmd):** run `install-remote.bat` from a clone, or use the PowerShell one-liner above.

Pin version: `AGNES_AIGC_VERSION=0.3.0` before the curl/irm command.

### Source install (developers)

From a git clone:

```bash
./install.sh
```

Builds with `cargo build --release`, installs to `~/.local/bin`, and installs the skill to `~/.agents/skills/agnes-aigc-gen/` by default.

Optional agent-specific installs:

```bash
INSTALL_AGENTS=cursor,claude,codex,openclaw,hermes ./install.sh
INSTALL_AGENTS=all ./install-remote.sh
```

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### Release tags (maintainers)

Bump `version` in `Cargo.toml`, then:

```bash
git tag v0.3.0
git push origin v0.3.0
```

Tag `v*` triggers multi-platform builds via `.github/workflows/release.yml`.

## Configure

```bash
agnes-aigc-gen config set api-key YOUR_API_KEY
agnes-aigc-gen config show
```

API key is encrypted in `{config_dir}/config.toml`. Full setup: [skills/agnes-aigc-gen/SETUP.md](skills/agnes-aigc-gen/SETUP.md). Command reference: [SKILL.md](skills/agnes-aigc-gen/SKILL.md).

**Defaults**

| Setting | Default | Notes |
|---------|---------|-------|
| `base_url` | `https://apihub.agnes-ai.com/v1` | Official Agnes API gateway |
| `text_model` | `agnes-2.0-flash` | Chat model |
| `thinking_text_model` | unset | Optional thinking model; falls back to `text_model` |
| `chat_thinking` | `true` | Default chat thinking mode |
| `chat_context_tokens` | `262144` | Chat context window |
| `chat_max_output_tokens` | `65536` | Chat max output |
| `output_dir` | `.` | Used with `--save` only |

## Usage

Every option has a long form (`--prompt`, `--ratio`, …). Short forms (`-p`, `-r`, …) are optional shortcuts — see [SKILL.md](skills/agnes-aigc-gen/SKILL.md) for the full table.

```bash
# Image (text-to-image)
agnes-aigc-gen image -p "A cat on the beach" --ratio 16:9

# Image-to-image — local path, URL, asset://, base64, or data URI
agnes-aigc-gen image -p "Cyberpunk style" --ratio 1:1 -i a.png,b.jpg

# Batch (2–4 concurrent); do not combine with --seed
agnes-aigc-gen image -p "portrait variants" --ratio 9:16 -n 4

# Video (text-to-video)
agnes-aigc-gen video -p "Cinematic walk" --ratio 16:9 -d 5

# Image → video: generate first, then pass asset:// (HTTPS URL only for -i)
agnes-aigc-gen image -p "Portrait, soft light" --ratio 9:16
agnes-aigc-gen video -p "Subtle motion" -d 3 \
  --negative-prompt "blurry, watermark" \
  -i asset://<id-from-json>

# Verbose polling logs
agnes-aigc-gen -v video -p "Ocean waves" --ratio 16:9 -d 5

# Async video: submit and return task_id immediately
agnes-aigc-gen video -p "Ocean waves" --ratio 16:9 -d 5 --async
agnes-aigc-gen task list              # recent tasks (default 10); refreshes in-progress
agnes-aigc-gen task list -n 20
agnes-aigc-gen task show 3              # by local id (or full vendor task_id)
agnes-aigc-gen task wait 3              # block until complete

# Dashboard
agnes-aigc-gen dashboard

# Agent chat TUI
agnes-aigc-gen chat
agnes-aigc-gen chat --prompt "Generate a portrait, then submit a short i2v video"
agnes-aigc-gen chat --no-thinking --context-tokens 128k --max-output-tokens 16k
agnes-aigc-gen chat --auto
```

### Image vs video inputs

| Input | Image `-i` / `--input` | Video `-i` / `--image` |
|-------|------------------------|-------------------------|
| Local path | ✓ | ✗ |
| HTTPS URL | ✓ | ✓ |
| `asset://` | ✓ | ✓ |
| base64 / data URI | ✓ | ✗ |

Video does not upload local files or call the image API to stage frames. Generate an image first and chain with `asset_uri` from JSON output.

### Key limits

| Topic | Rule |
|-------|------|
| Ratios | `1:1`, `4:3`, `3:4`, `16:9`, `9:16` |
| Image seed | `-s` / `--seed`, 0–999; mutually exclusive with `-n > 1` |
| Video seed | `-s` / `--seed`, 0–999; only sent when set |
| Video duration | max `floor(441 / frame_rate)` seconds (18s @ 24 fps) |
| Video frame rate | `-f` / `--frame-rate`, 1–60 (default 24) |
| Negative prompt | `--np` / `--negative-prompt` on video (top-level API field) |
| Video async | `--async` submits task and returns local `id` + vendor `task_id`; use `task list` / `task show 3` |

### Agent chat

`agnes-aigc-gen chat` launches a PI-based terminal agent. It can use PI coding tools (`read`, `write`, `edit`, `bash`, `web_fetch`, `todo`) plus Agnes tools for image generation, async video submission, assets, history, tasks, and skill loading.

Approval modes:

| Mode | Behavior |
|------|----------|
| default | Reviews side-effecting tools and media generation |
| `--auto` | Auto-approves non-dangerous calls |
| dangerous commands | Always require human approval |

Dangerous examples include force push, retagging, `git reset --hard`, `git clean`, `rm -rf`, `sudo`, `curl | sh`, publish/release commands, and writes outside the workspace.

Useful slash commands in the TUI: `/help`, `/new`, `/sessions`, `/resume <id>`, `/skills`, `/skill <name>`, `/tools`, `/approval`, `/model`, `/thinking`, `/tasks`, `/retry`, `/quit`.

TUI keys: `Tab` completes slash commands, `Ctrl-R` retries the previous request without adding another user message to session history, `Ctrl-T` toggles thinking details, `Ctrl-E` toggles tool-call details, and `Ctrl-Q` quits. Thinking and tool details are folded by default to keep the conversation readable.

Chat completion requests automatically try up to 3 total attempts for retryable network failures, `408`, `429`, and `5xx` responses before surfacing an error.

Video tool calls submit asynchronously by default and return a local task id. Use `/tasks` or the regular `task list/show/wait` commands to follow progress.

See `agnes-aigc-gen --help`, [SKILL.md](skills/agnes-aigc-gen/SKILL.md) (usage), and [SETUP.md](skills/agnes-aigc-gen/SETUP.md) (install & config).

## API reference docs

Local copies of the Agnes official docs (Chinese, for reference):

| Doc | Model |
|-----|-------|
| [docs/agnes-image-2.1-flash.md](docs/agnes-image-2.1-flash.md) | Image |
| [docs/agnes-video-v2.0.md](docs/agnes-video-v2.0.md) | Video |
| [docs/agnes-2.0-flash.md](docs/agnes-2.0-flash.md) | Text (chat) |

Source: [agnes-ai.com/doc](https://agnes-ai.com/doc)

## Output

Default JSON (`uri` is remote URL):

```json
{
  "type": "image",
  "ratio": "16:9",
  "size": "1280x720",
  "uri": "https://storage.googleapis.com/.../image.png",
  "asset_uri": "asset://abc123"
}
```

Batch image (`-n 2–4`):

```json
{
  "results": [
    { "success": true, "type": "image", "uri": "https://...", "asset_uri": "asset://abc" },
    { "success": false, "message": "..." }
  ]
}
```

| Flag | Effect |
|------|--------|
| `--save` | Download to `output_dir` |
| `--output-format plain` | Print `uri` only |
| `--retries` | API retry count |
| `-v` / `--verbose` | Debug logs on stderr |

## Development

```bash
cargo build
cargo test
cargo run -- config show
```

## Project layout

```
docs/                    # Agnes API reference (image, video, text)
scripts/                 # install-skill.sh / install-skill.ps1
skills/agnes-aigc-gen/   # SKILL.md (usage) + SETUP.md (install & config)
src/cli/                 # image, video, task, config, dashboard, chat
src/ui/                  # ratatui dashboard + chat TUI
src/api/                 # Agnes HTTP client
install.sh               # source: cargo build + install
install-remote.sh        # release: download from GitHub Releases (Unix)
install-remote.ps1       # release: download from GitHub Releases (Windows)
install-remote.bat       # Windows wrapper for install-remote.ps1
```

## License

See repository license file if present.
