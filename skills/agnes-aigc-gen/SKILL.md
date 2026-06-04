---
name: agnes-aigc-gen
description: >-
  Generates images and videos through the Agnes AI API via the agnes-aigc-gen CLI.
  Use when the user wants Agnes image/video generation, batch images, image-to-video
  with asset:// links, API key configuration, or agnes-aigc-gen commands in agent workflows.
  Read this skill before calling the CLI to avoid invalid flags, ratio/duration errors,
  wrong output assumptions, or missing file dependencies (config.toml, generations.db,
  input paths, platform-specific config directory).
---

# agnes-aigc-gen

CLI for [Agnes AI](https://agnes-ai.com) image and video generation (OpenAI-compatible gateway).

## Quick checklist (read before every call)

| Rule | Detail |
|------|--------|
| **Never pass `--size`** | Use `--ratio` only; pixel size is derived automatically |
| **Image ratios** | Only `1:1`, `4:3`, `3:4`, `16:9`, `9:16` |
| **Video images flag** | Use `-i` / `--image`, **not** `--first-frame`, `--keyframes`, or `--input` |
| **Batch count `-n`** | **Image only**, integer **1–4**; not supported on video |
| **Seed** | Image only; **0–999** if set; omit for random per call |
| **Default output** | `uri` = **remote HTTPS URL**; no local download unless `--save` |
| **Video duration cap** | `max_seconds = floor(441 / frame_rate)`; default fps **24** → max **18s** |
| **Video prompt** | Required unless polling with `--task-id` |
| **Chain image → video** | Pass prior `asset_uri` as `-i asset://<id>` |
| **Verbose logs** | Only with global `-v` / `--verbose`; otherwise silent on stderr |
| **Do not rely on `config save-local`** | Image/video download is controlled only by `--save` on the command |

## Prerequisites

```bash
agnes-aigc-gen config set api-key YOUR_API_KEY
agnes-aigc-gen config show
```

- Config dir: see **File dependencies** (macOS ≠ `~/.config`)
- Default base URL: `https://apihub.agnes-ai.com/v1`
- History DB: `{config_dir}/generations.db`

Re-set API key after machine change if decryption fails.

## File dependencies

Agents must understand which paths are read, written, or required at runtime. **Do not assume `~/.config` on every OS** — the CLI uses the platform config directory from the `dirs` crate.

### Config & state directory

| OS | Path |
|----|------|
| macOS | `~/Library/Application Support/agnes-aigc-gen/` |
| Linux | `~/.config/agnes-aigc-gen/` |
| Windows | `%APPDATA%\agnes-aigc-gen\` (Roaming) |

Created automatically on first run (`config set`, generation, or DB access).

### Files managed by the CLI

| File | Access | Purpose |
|------|--------|---------|
| `{config_dir}/config.toml` | read / write | Settings + **encrypted** `api_key_encrypted` (machine-bound; **no** `machine_id` stored) |
| `{config_dir}/generations.db` | read / write | SQLite: `assets` (id → remote URL), `generations` (history) |

**`asset://` resolution** reads `generations.db` only. If the DB is missing, deleted, or the id was never recorded, `asset://…` lookups fail.

### Machine binding (runtime OS reads, not config files)

API key encryption/decryption derives a key from the **current machine** at runtime. These OS sources are read when encrypting or decrypting; they are **never written to `config.toml`**:

| OS | Runtime identity source |
|----|-------------------------|
| macOS | `IOPlatformUUID` via `ioreg` |
| Linux | `/etc/machine-id` or `/var/lib/dbus/machine-id` |
| Windows | Registry `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid` |

Copying `config.toml` to another machine **does not** transfer a usable API key.

### Binary & skill (install)

| Path | Role |
|------|------|
| `~/.local/bin/agnes-aigc-gen` | Installed binary (`./install.sh`; must be on `PATH`) |
| `~/.cursor/skills/agnes-aigc-gen/SKILL.md` | Cursor skill (optional; copied by `install.sh`) |

Repo-local build: `target/release/agnes-aigc-gen` (via `cargo build --release`).

### User input files (`-i` / `--input` / `--image`)

| Input | Requirement |
|-------|-------------|
| Local path | Must **exist** and be readable (png, jpeg, webp, etc.) |
| `https://…` URL | Network access; fetched at runtime |
| `asset://<id>` | Id must exist in **`generations.db`** |
| `base64:` / data URI | Inline payload; no file needed |

For **video**, prefer `asset://` or HTTPS URL from a prior image run. Local paths trigger extra image-API staging (slower, more failure modes).

### Local output files (`--save` only)

Default: **no local files** — JSON `uri` is the remote URL.

With `--save` (and optional `--output-dir`):

| Setting | Resolved directory |
|---------|------------------|
| `config output-dir` default `.` | Current working directory when the command runs |
| `~/path` | Expanded home directory |
| `--output-dir PATH` | Overrides config for that command |

Downloaded files are named `{YYYYMMDD-HHMMSS}-{sha256}.{ext}` (e.g. `.png`, `.mp4`) under the resolved output directory. Directory is created if missing.

`config save-local` does **not** enable downloads; only `--save` on `image` / `video` does.

### Network dependencies

| Endpoint | When |
|----------|------|
| `{base_url}/images/generations` | Every image call |
| `{base_url}/videos` + `GET /videos/{task_id}` | Video create + poll |
| Remote URLs in `uri` / `assets.remote_url` | Default output; video frame inputs |

Default `base_url`: `https://apihub.agnes-ai.com/v1`.

### Agent file checklist

Before calling the CLI, verify:

1. **`agnes-aigc-gen` on PATH** (or use full path to binary)
2. **`config.toml` exists** with `api_key_encrypted` (`config set api-key …`)
3. **Input paths exist** when using local `-i` files
4. **`asset://` ids** come from a prior successful generation on **this machine** (same `generations.db`)
5. **Do not expect local output files** unless `--save` was passed
6. **Video long-running** — do not delete or move `generations.db` mid-workflow if chaining via `asset://`
7. **macOS agents** — use `~/Library/Application Support/agnes-aigc-gen/`, not `~/.config/…`


```bash
agnes-aigc-gen -v image ...    # debug logs on stderr
agnes-aigc-gen --verbose video ...
```

Without `-v`, the CLI prints **only JSON/plain result on stdout** (no progress logs).

---

## Image generation

### Basic usage

```bash
# Text-to-image (default ratio 1:1)
agnes-aigc-gen image -p "A cat on the beach" --ratio 16:9

# Image-to-image / edit
agnes-aigc-gen image -p "Make it cyberpunk" --ratio 9:16 \
  -i ./photo.png -i https://example.com/ref.jpg

# Comma-separated inputs
agnes-aigc-gen image -p "Blend styles" --ratio 1:1 -i ./a.png,./b.jpg
```

### Supported aspect ratios & output sizes

| Ratio | Size (image) |
|-------|----------------|
| `1:1` | 1024×1024 |
| `4:3` | 1152×864 |
| `3:4` | 864×1152 |
| `16:9` | 1280×720 |
| `9:16` | 720×1280 |

Any other ratio string **errors**. Do not invent ratios like `2:3` unless you verify support first.

### Input formats (`-i` / `--input`)

| Form | Example |
|------|---------|
| Local path | `./photo.png` |
| HTTP(S) URL | `https://example.com/a.jpg` |
| Prior asset | `asset://c8d4eb63a84b` |
| Base64 prefix | `base64:...` or raw base64 string |
| Data URI | `data:image/jpeg;base64,...` |

- Non-JPEG inputs are converted to `data:image/jpeg;base64,...` for the API
- JPEG files/data URIs may be sent as-is

### Batch generation (`-n`, image only)

```bash
agnes-aigc-gen image -p "portrait" --ratio 9:16 -n 4
```

| Constraint | Value |
|------------|-------|
| `-n` range | **1–4** (values outside range error) |
| Execution | **Concurrent** API calls |
| Partial failure | Allowed; failed items include `"success": false` and `"message"` |
| Exit code | **0** if at least one success; **1** if all fail |

**Single image (`-n 1` or omitted)** — stdout:

```json
{
  "type": "image",
  "ratio": "9:16",
  "size": "720x1280",
  "uri": "https://storage.googleapis.com/.../image.png",
  "asset_uri": "asset://c8d4eb63a84b",
  "generation_id": 1
}
```

**Batch (`-n 2`–`4`)** — stdout:

```json
{
  "results": [
    {
      "success": true,
      "type": "image",
      "ratio": "9:16",
      "size": "720x1280",
      "uri": "https://...",
      "asset_uri": "asset://abc",
      "generation_id": 2
    },
    {
      "success": false,
      "message": "image generation failed (429): ..."
    }
  ]
}
```

Parse **`results`** array for batch; parse top-level object for single.

### Image flags

| Flag | Default | Notes |
|------|---------|-------|
| `-p` / `--prompt` | required | |
| `-r` / `--ratio` | `1:1` | Must be supported ratio |
| `-n` / `--count` | `1` | Max **4**, image only |
| `--seed` | random 0–999 | Must be **0–999** if set |
| `-i` / `--input` | none | Repeatable |
| `--save` | off | Download to `--output-dir` or config `output_dir` |
| `--output-dir` | config | Used with `--save` |
| `--output-format` | `json` | `plain` prints `uri` lines only |
| `--retries` | config (3) | API/download retries |

---

## Video generation

### Basic usage

```bash
# Text-to-video
agnes-aigc-gen video -p "Ocean waves at sunset" --ratio 16:9 -d 5

# Image-to-video (use asset from prior image gen)
agnes-aigc-gen video -p "Gentle camera motion" --ratio 9:16 -d 3 \
  -i asset://c8d4eb63a84b

# Keyframes (exactly 2 images)
agnes-aigc-gen video -p "Smooth transition" -d 5 \
  -i ./start.jpg -i ./end.jpg

# Resume / poll existing task
agnes-aigc-gen video --task-id task_xxxxxxxx
```

### Video input modes (`-i` / `--image`)

| # of images | Mode |
|-------------|------|
| 0 | Text-to-video (`--ratio` required) |
| 1 | Image-to-video |
| 2 | Keyframes |
| 3+ | Multi-frame |

**All input frames must share the same aspect ratio.** When images are provided, output dimensions follow the **image ratio**, not necessarily `--ratio` (still pass matching `--ratio` for clarity).

### Video API input rules (avoid runtime errors)

1. **Video endpoint accepts HTTP(S) URLs** for frame images — not raw base64 in the video API body
2. **`asset://` IDs** are resolved to stored remote URLs automatically
3. **Local files / base64** are staged: cropped to JPEG → uploaded via a one-off image API call → URL passed to video API
4. Do **not** pass local paths to video if the staging image API might fail; prefer **`asset://` or HTTPS URL** from a prior successful image generation

### Duration & frame rate limits

Frame count sent to API follows **8n+1** snapping, max **441 frames**.

**Maximum requested duration (seconds):**

```
max_duration = floor(441 / frame_rate)
```

| frame_rate | max `-d` |
|------------|----------|
| 24 (default) | **18** |
| 30 | **14** |
| 25 | **17** |

- `-d` must be **> 0** and **≤ max_duration** or CLI errors before API call
- Example error: `duration 20s exceeds maximum 18s at 24 fps (max 441 frames)`
- Actual encoded length may differ slightly after frame snapping; CLI validates **requested** duration only

```bash
agnes-aigc-gen video -p "..." --ratio 9:16 -d 3 --frame-rate 24   # OK
agnes-aigc-gen video -p "..." --ratio 9:16 -d 20 --frame-rate 24  # ERROR
```

### Supported video sizes (720p tier)

| Ratio | Size (video) |
|-------|----------------|
| `1:1` | 768×768 |
| `4:3` | 960×768 |
| `3:4` | 768×960 |
| `16:9` | 1280×768 |
| `9:16` | 768×1280 |

### Polling behavior

After `POST /videos`, CLI polls `GET /videos/{task_id}` until `completed` or `failed`:

| Elapsed since start | Poll interval |
|---------------------|---------------|
| First 2 minutes | **30s** |
| After 2 minutes | **15s** |

Video jobs often stay `queued` for several minutes — **wait for the command to finish**; do not assume failure while exit code is pending.

Use `-v` to see poll status on stderr.

### Video output (success)

```json
{
  "type": "video",
  "ratio": "9:16",
  "size": "768x1280",
  "uri": "https://storage.googleapis.com/.../video.mp4",
  "asset_uri": "asset://eb796809aa05",
  "generation_id": 2
}
```

- Default `uri` = remote MP4 URL
- Add `--save` to download locally; then `uri` = local file path

### Video flags

| Flag | Default | Notes |
|------|---------|-------|
| `-p` / `--prompt` | required* | *Optional only with `--task-id` |
| `-r` / `--ratio` | `16:9` | Used for text-to-video; should match input image ratio for i2v |
| `-d` / `--duration` | `5` | Must respect max duration table |
| `--frame-rate` | `24` | Must be > 0 |
| `-i` / `--image` | none | Repeatable; **not** `--input` |
| `--task-id` | none | Poll existing task |
| `--save` | off | Download MP4 locally |
| `--output-dir` | config | Used with `--save` |
| `--output-format` | `json` | `plain` → uri only |
| `--retries` | config (3) | HTTP retry for API calls |

---

## Recommended agent workflow (image → video)

```bash
# Step 1: generate image
agnes-aigc-gen image -p "Portrait of a woman, soft light" --ratio 9:16

# Step 2: parse JSON → asset_uri (e.g. asset://c8d4eb63a84b)

# Step 3: image-to-video (match ratio, duration within cap)
agnes-aigc-gen -v video -p "Subtle motion, cinematic" \
  --ratio 9:16 -d 3 -i asset://c8d4eb63a84b
```

**Always:**

1. Use the **`asset_uri`** from step 1 for step 3 (not the local path from `--save`)
2. Match **`--ratio`** between image and video
3. Keep **`-d` ≤ 18`** at default 24 fps
4. Allow **several minutes** for video completion

---

## Assets & history

```bash
agnes-aigc-gen asset list
agnes-aigc-gen asset show c8d4eb63a84b
agnes-aigc-gen asset show asset://c8d4eb63a84b

agnes-aigc-gen history list
agnes-aigc-gen history show 1
```

Use when `asset://` lookup fails or to inspect stored remote URLs.

---

## Configuration

```bash
agnes-aigc-gen config set base-url https://apihub.agnes-ai.com/v1
agnes-aigc-gen config set image-model agnes-image-2.1-flash
agnes-aigc-gen config set video-model agnes-video-v2.0
agnes-aigc-gen config set text-model agnes-2.0-flash
agnes-aigc-gen config set output-dir .
agnes-aigc-gen config set max-retries 3
agnes-aigc-gen config show
```

| Key | Default |
|-----|---------|
| `base-url` | `https://apihub.agnes-ai.com/v1` |
| `image-model` | `agnes-image-2.1-flash` |
| `video-model` | `agnes-video-v2.0` |
| `output-dir` | `.` |
| `max-retries` | `3` |

`config set save-local` does **not** change image/video CLI download behavior; use `--save` on each command.

---

## Models & endpoints

| Type | Model | Endpoint |
|------|-------|----------|
| Image | `agnes-image-2.1-flash` | `POST /v1/images/generations` |
| Video | `agnes-video-v2.0` | `POST /v1/videos` + poll `GET /v1/videos/{id}` |
| Text (future) | `agnes-2.0-flash` | `POST /v1/chat/completions` |

---

## Common errors & fixes

| Error / symptom | Cause | Fix |
|-----------------|-------|-----|
| `API key not configured` | Missing key | `config set api-key ...` |
| Machine ID / decrypt error | New machine or copied config | Re-set API key on this machine (`config set api-key …`) |
| `invalid ratio` / unsupported image ratio | Bad `--ratio` | Use one of five supported ratios |
| `count must be 1–4` | `-n` on image | Use 1–4 only; no `-n` on video |
| `seed must be 0–999` | Bad `--seed` | Clamp or omit |
| `duration ... exceeds maximum` | `-d` too long | `floor(441/fps)`; e.g. max 18s @ 24fps |
| `--prompt is required` | Video without prompt | Add `-p` unless `--task-id` |
| `asset not found: asset://...` | Missing DB row or wrong machine | Regenerate image; check `{config_dir}/generations.db` via `asset list` |
| Video `cannot identify image file` | Base64/local sent directly to video API | Use `asset://` or HTTPS URL from prior image |
| Input frames ratio mismatch | Mixed aspect ratios | Same ratio on all `-i` images |
| Batch all failed | API/rate limit | Check `message` in failed `results[]` items |
| Empty output / hung command | Video still queued | Wait; use `-v` to monitor; poll with `--task-id` if needed |
| Expected local file but got URL | Default is remote | Add `--save` to download |

---

## Deprecated / invalid flags (do not use)

These will **not** work or are removed:

- `--size` on image or video
- `--no-save` (use default remote URL, or `--save` to download)
- `--first-frame`, `--keyframes` on video (use `-i` / `--image` instead)
- `--input` on video (image command only)
- `-n` on video
- `remote_url` field in JSON output (use `uri` only)
- Relying on `config save-local true` for automatic download

---

## Other commands

```bash
agnes-aigc-gen dashboard   # ratatui UI
agnes-aigc-gen chat        # not implemented yet
```

For agent automation, prefer **`image`** and **`video`** subcommands with JSON stdout parsing.
