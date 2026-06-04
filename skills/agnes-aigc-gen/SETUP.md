# agnes-aigc-gen — Agent setup

Guide for **installing the CLI and skill** and **initial configuration**.  
For command usage, limits, and runtime file dependencies, see [SKILL.md](./SKILL.md).

Repository: **https://github.com/Zander-1024/agnes-aigc-gen**

---

## 1. Install the CLI

### Option A — Release install (recommended)

Downloads a prebuilt binary from [GitHub Releases](https://github.com/Zander-1024/agnes-aigc-gen/releases). Uses the **latest** release unless `AGNES_AIGC_VERSION` is set.

**macOS / Linux:**

```bash
curl -fsSL https://raw.githubusercontent.com/Zander-1024/agnes-aigc-gen/main/install-remote.sh | bash
```

**Pin a version:**

```bash
AGNES_AIGC_VERSION=0.1.0 curl -fsSL https://raw.githubusercontent.com/Zander-1024/agnes-aigc-gen/main/install-remote.sh | bash
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/Zander-1024/agnes-aigc-gen/main/install-remote.ps1 | iex
```

**Windows (cmd, from cloned repo):**

```bat
install-remote.bat
```

| Env var | Default | Purpose |
|---------|---------|---------|
| `AGNES_AIGC_VERSION` | latest release | e.g. `0.1.0` or `v0.1.0` |
| `AGNES_AIGC_REPO` | `Zander-1024/agnes-aigc-gen` | Override repo |
| `INSTALL_BIN_DIR` | `~/.local/bin` (Unix) / `%USERPROFILE%\.local\bin` (Windows) | Binary install path |
| `INSTALL_SKILL_DIR` | (unset) | If set, install skill **only** to this directory (overrides default) |
| `INSTALL_AGENTS` | (unset) | Comma-separated agents to also install: `cursor`, `claude`, `codex`, `openclaw`, `hermes`, or `all` |
| `SKIP_SKILL=1` | (unset) | Set to skip skill install |

Default skill path: **`~/.agents/skills/agnes-aigc-gen/`** (agent-agnostic). Use `INSTALL_AGENTS` to copy to agent-specific dirs (see below).

Release artifact URL pattern:

```
https://github.com/Zander-1024/agnes-aigc-gen/releases/download/v{VERSION}/agnes-aigc-gen-{VERSION}-{platform}.{tar.gz|zip}
```

Platforms: `linux-x86_64`, `linux-aarch64`, `darwin-x86_64`, `darwin-aarch64`, `windows-x86_64`.

### Option B — Source install (developers)

Requires Rust 1.85+. From a git clone:

```bash
git clone https://github.com/Zander-1024/agnes-aigc-gen.git
cd agnes-aigc-gen
./install.sh
```

`install.sh` runs `cargo build --release`, installs the binary to `~/.local/bin`, and installs the skill to `~/.agents/skills/agnes-aigc-gen/` by default.

### PATH

Ensure the install directory is on `PATH`:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Verify:

```bash
agnes-aigc-gen --help
```

---

## 2. Install the agent skill

Release/source install scripts install the skill automatically.

### Default path (always)

```
~/.agents/skills/agnes-aigc-gen/
├── SKILL.md    # command reference
└── SETUP.md    # this file
```

### Optional: agent-specific paths

Set `INSTALL_AGENTS` to also install copies for your coding agent:

| Agent | Path |
|-------|------|
| (default) | `~/.agents/skills/agnes-aigc-gen/` |
| Cursor | `~/.cursor/skills/agnes-aigc-gen/` |
| Claude Code | `~/.claude/skills/agnes-aigc-gen/` |
| Codex | `~/.codex/skills/agnes-aigc-gen/` |
| OpenClaw | `~/.openclaw/skills/agnes-aigc-gen/` |
| Hermes | `~/.hermes/skills/agnes-aigc-gen/` |

Examples:

```bash
# Default only (~/.agents/skills/)
./install.sh

# Default + Cursor + Claude
INSTALL_AGENTS=cursor,claude ./install.sh

# Default + all supported agents
INSTALL_AGENTS=all ./install-remote.sh

# Custom single directory only (legacy override)
INSTALL_SKILL_DIR=~/.cursor/skills ./install-remote.sh
```

Skip skill install: `SKIP_SKILL=1 ./install-remote.sh`

**Manual install (default path):**

```bash
mkdir -p ~/.agents/skills/agnes-aigc-gen
curl -fsSL https://raw.githubusercontent.com/Zander-1024/agnes-aigc-gen/main/skills/agnes-aigc-gen/SKILL.md \
  -o ~/.agents/skills/agnes-aigc-gen/SKILL.md
curl -fsSL https://raw.githubusercontent.com/Zander-1024/agnes-aigc-gen/main/skills/agnes-aigc-gen/SETUP.md \
  -o ~/.agents/skills/agnes-aigc-gen/SETUP.md
```

**Project-only:** copy `skills/agnes-aigc-gen/` to `.cursor/skills/`, `.claude/skills/`, or your agent's project skills folder.

---

## 3. Configure API key (required)

Get a key from [platform.agnes-ai.com](https://platform.agnes-ai.com):

```bash
agnes-aigc-gen config set api-key YOUR_API_KEY
agnes-aigc-gen config show
```

- Stored **encrypted** in `{config_dir}/config.toml` (machine-bound, not plaintext)
- Re-run `config set api-key` on a new machine or if decryption fails
- Never commit `config.toml` or share `api_key_encrypted`

Optional settings:

```bash
agnes-aigc-gen config set base-url https://apihub.agnes-ai.com/v1
agnes-aigc-gen config set output-dir .
agnes-aigc-gen config set max-retries 3
```

---

## 4. Configuration reference (`config.toml`)

**Location** (platform-specific `{config_dir}`):

| OS | Path |
|----|------|
| macOS | `~/Library/Application Support/agnes-aigc-gen/config.toml` |
| Linux | `~/.config/agnes-aigc-gen/config.toml` |
| Windows | `%APPDATA%\agnes-aigc-gen\config.toml` |

Created/updated by `agnes-aigc-gen config set …`. Do not hand-edit `api_key_encrypted`.

### Example (after `config set api-key`)

```toml
base_url = "https://apihub.agnes-ai.com/v1"
text_model = "agnes-2.0-flash"
image_model = "agnes-image-2.1-flash"
video_model = "agnes-video-v2.0"
output_dir = "."
save_local = false
max_retries = 3
api_key_encrypted = "<base64 ciphertext; machine-bound>"
```

### Fields

| Field | Default | Description |
|-------|---------|-------------|
| `base_url` | `https://apihub.agnes-ai.com/v1` | API gateway |
| `text_model` | `agnes-2.0-flash` | Text model (future) |
| `image_model` | `agnes-image-2.1-flash` | Image model |
| `video_model` | `agnes-video-v2.0` | Video model |
| `output_dir` | `.` | Base dir for `--save` downloads |
| `save_local` | `false` | Legacy; does **not** auto-download CLI output |
| `max_retries` | `3` | HTTP retries |
| `api_key_encrypted` | (none) | Set via `config set api-key` |

### `config set` keys

| CLI key | Config field |
|---------|--------------|
| `api-key` | `api_key_encrypted` |
| `base-url` | `base_url` |
| `text-model` | `text_model` |
| `image-model` | `image_model` |
| `video-model` | `video_model` |
| `output-dir` | `output_dir` |
| `save-local` | `save_local` |
| `max-retries` | `max_retries` |

### Machine binding

API key encryption uses the **current machine's OS identity** at runtime (not stored in config):

| OS | Source |
|----|--------|
| macOS | `IOPlatformUUID` |
| Linux | `/etc/machine-id` |
| Windows | `MachineGuid` registry |

Copying `config.toml` to another machine does not transfer a usable key.

---

## 5. Post-setup checklist

Before using the CLI in agent workflows:

- [ ] `agnes-aigc-gen` on `PATH`
- [ ] `config set api-key` completed on this machine
- [ ] `config show` shows `api_key` as `<configured>`
- [ ] Skill installed at `~/.agents/skills/agnes-aigc-gen/SKILL.md` (or agent path via `INSTALL_AGENTS`)
- [ ] Read [SKILL.md](./SKILL.md) for command rules and runtime dependencies

Smoke test:

```bash
agnes-aigc-gen image -p "test" --ratio 1:1
```

---

## 6. Release & tags (maintainers)

Pushing a **`v*`** tag triggers `.github/workflows/release.yml`:

1. Bump `version` in `Cargo.toml`
2. `git tag v0.1.0 && git push origin v0.1.0`
3. CI publishes multi-platform Release assets

Tag version must match `Cargo.toml` `version`.

---

## 7. Troubleshooting setup

| Problem | Fix |
|---------|-----|
| `command not found` | Add `INSTALL_BIN_DIR` to `PATH` |
| `API key not configured` | `config set api-key …` |
| Decrypt / machine error | Re-set API key on this machine |
| Release install fails | Ensure a GitHub Release exists for the requested version |
| Skill not loaded | Install to `~/.agents/skills/agnes-aigc-gen/` or set `INSTALL_AGENTS=cursor,claude,...` |

Command-level errors: see **Common errors** in [SKILL.md](./SKILL.md).
