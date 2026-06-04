# agnes-aigc-gen

CLI and terminal dashboard for [Agnes AI](https://agnes-ai.com) image and video generation.

## Features

- **Image generation** — text-to-image and image-to-image (`agnes-image-2.1-flash`)
- **Video generation** — text-to-video, first-frame, keyframes, multi-frame (`agnes-video-v2.0`)
- **Aspect ratios** — pass `--ratio` only; dimensions computed internally
- **Structured output** — JSON with `ratio`, `size`, `uri`, `remote_url`; default local download
- **Encrypted config** — API key stored machine-bound in `~/.config/agnes-aigc-gen/`
- **Dashboard** — `agnes-aigc-gen dashboard` (ratatui terminal UI)
- **Agent skill** — Cursor skill under `skills/agnes-aigc-gen/`

## Requirements

- Rust 1.85+ (edition 2024)
- Agnes API key from [platform.agnes-ai.com](https://platform.agnes-ai.com)

## Install

### Quick install (recommended)

From the repository root:

```bash
./install.sh
```

This will:

1. Build a release binary with `cargo build --release`
2. Install the binary to **`~/.local/bin/agnes-aigc-gen`**
3. Install the Cursor skill to **`~/.cursor/skills/agnes-aigc-gen/`**

Ensure `~/.local/bin` is on your `PATH`. Add to `~/.zshrc` or `~/.bashrc` if needed:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### Manual install

```bash
cargo build --release
mkdir -p ~/.local/bin
cp target/release/agnes-aigc-gen ~/.local/bin/
chmod +x ~/.local/bin/agnes-aigc-gen
```

### Install the Cursor skill

**Option A — user-wide (recommended for agents across projects):**

```bash
mkdir -p ~/.cursor/skills
cp -R skills/agnes-aigc-gen ~/.cursor/skills/
```

**Option B — project-only (this repo):**

```bash
mkdir -p .cursor/skills
cp -R skills/agnes-aigc-gen .cursor/skills/
```

The skill lives at `skills/agnes-aigc-gen/SKILL.md` in this repository. Cursor discovers skills from `~/.cursor/skills/<name>/SKILL.md` or `.cursor/skills/<name>/SKILL.md`.

## Configure

```bash
agnes-aigc-gen config set api-key YOUR_API_KEY
agnes-aigc-gen config set output-dir .    # optional; default is current directory
agnes-aigc-gen config show
```

**Defaults**

| Setting | Default | Notes |
|---------|---------|-------|
| `base_url` | `https://apihub.agnes-ai.com/v1` | Official Agnes API gateway (OpenAI-compatible) |
| `output_dir` | `.` | Downloads go to the **current working directory** |

## Usage

```bash
# Image
agnes-aigc-gen image -p "A cat on the beach" --ratio 16:9

# Image-to-image (multiple inputs)
agnes-aigc-gen image -p "Cyberpunk style" --ratio 1:1 --input a.png,b.jpg

# Video
agnes-aigc-gen video -p "Cinematic walk" --ratio 16:9 --duration 5

# Dashboard
agnes-aigc-gen dashboard
```

See `agnes-aigc-gen --help` and [skills/agnes-aigc-gen/SKILL.md](skills/agnes-aigc-gen/SKILL.md) for full reference.

## Output

Default JSON (auto-download enabled):

```json
{
  "type": "image",
  "ratio": "16:9",
  "size": "1366x768",
  "uri": "/Users/you/projects/20260603-143022-abc.png",
  "remote_url": "https://..."
}
```

| Flag | Effect |
|------|--------|
| `--output-dir` | Override save directory |
| `--no-save` | Return remote URL only |
| `--output-format plain` | Print `uri` only |
| `--retries` | API/download retry count |

## Development

```bash
cargo build
cargo test
cargo run -- config show
```

## Project layout

```
skills/agnes-aigc-gen/   # Cursor agent skill (English)
src/cli/                 # image, video, config, dashboard, chat
src/ui/                  # ratatui dashboard
src/api/                 # Agnes HTTP client
install.sh               # build + install binary and skill
```

## License

See repository license file if present.
