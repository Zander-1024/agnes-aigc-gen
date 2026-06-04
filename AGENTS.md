# Agent guide — agnes-aigc-gen

Instructions for AI agents and maintainers working in this repository.

## Before commit or push

Run the same checks as [`.github/workflows/ci.yml`](.github/workflows/ci.yml). **Do not commit or push until all steps pass.**

```bash
./scripts/ci-local.sh
```

Or run manually:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

## Commit and push (default)

After `./scripts/ci-local.sh` passes:

```bash
git push origin master
```

**Do not** force-push tags, retag, or re-trigger Release unless the user **explicitly** asks (e.g. “重新打 tag 发版”).

## Release (maintainers, only when requested)

Only when the user asks for a version release:

1. Bump `Cargo.toml` `version` to match the new tag (e.g. `0.2.1` → `v0.2.1`).
2. Run `./scripts/ci-local.sh` on the release commit.
3. Create a **new** tag and push:

```bash
git tag v0.2.1
git push origin master
git push origin v0.2.1
```

Pushing a new `v*` tag triggers [`.github/workflows/release.yml`](.github/workflows/release.yml) (multi-platform binaries + GitHub Release).

**Retag / force-push** (`git tag -f`, `git push --force`) — only if the user explicitly requests re-releasing an existing tag after a failed build. Never do this proactively.

## Project conventions

- **Scope:** minimal diffs; match existing Rust/CLI patterns.
- **Video `-i`:** HTTPS URL or `asset://` only (no local paths for video).
- **Image `-i`:** local path, URL, `asset://`, base64, data URI.
- **Async video:** `video --async` → local SQLite `id` + vendor `task_id`; `task list` / `task show <id>` / `task wait <id>`.
- **Docs:** user-facing CLI usage in [`skills/agnes-aigc-gen/SKILL.md`](skills/agnes-aigc-gen/SKILL.md); install/config in [`SETUP.md`](skills/agnes-aigc-gen/SETUP.md).
- **Do not commit:** `.env`, API keys, or local smoke-test logs.

## Key paths

| Path | Purpose |
|------|---------|
| `src/api/` | Agnes HTTP client (image, video, poll) |
| `src/cli/` | Clap subcommands |
| `src/db/` | SQLite assets, generations, video_tasks |
| `skills/agnes-aigc-gen/` | Cursor/agent skill |
| `docs/agnes-*.md` | API reference copies |
