# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.4.1] - 2026-06-06

### Added

- Full ratatui **dashboard** for image/video generation, tasks, assets, and settings
- Scheme B layout: multiline prompt, inline params, references, bottom Output panel
- Async video tasks with in-page progress, task strip, and `[RUN N]` on Tasks page
- Tab cycles Prompt / Params / References / Output; F5 submit

### Changed

- Replaced legacy `ui/app.rs` stub dashboard with `ui/dashboard/` module
- Video async submissions return `generation_id` from local task record
- Status bar on generate pages shows brief text only (progress in Output panel)

## [0.4.0] - 2026-06-06

### Added

- `version` subcommand with `check` and `changelog`
- `self update` and `self uninstall` for release installs
- Root `-v` / `-V` prints version; `--verbose` is long-form only
- `install.toml` records binary and skill install locations

### Changed

- Install and config guide lives at `docs/SETUP.md` (skill install still copies `SETUP.md` beside `SKILL.md`)

## [0.3.2] - 2026-06-06

### Added

- Interactive `task list` and `asset list` TUIs with split Detail panel
- Left/Right field tabs; detail values truncated to 200 chars; Enter copies full text
- Shared `list_tui` helpers for list views

### Changed

- Maintainer release tag instructions moved from README to AGENTS.md

## [0.3.1] - 2026-06-05

### Added

- Interactive `task list` TUI with row selection and detail copy
- Video task progress column and API refresh for in-progress tasks

## [0.3.0] - 2026-06-04

### Added

- Agent chat TUI with PI tools, Agnes media tools, approvals, and session resume
- Async video submission via `video --async` and `task` subcommands
- Asset library (`asset://`) and generation history in SQLite

### Changed

- Video query id prefers Agnes `video_id` endpoint

## [0.2.0] - 2026-05-30

### Added

- Image and video generation CLI with structured JSON output
- Encrypted API key storage and terminal dashboard
- GitHub Release install scripts for macOS, Linux, and Windows
