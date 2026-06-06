#!/usr/bin/env bash
# Install agnes-aigc-gen skill to ~/.agents/skills/ and optional agent-specific dirs.
# Sourced by install.sh / install-remote.sh — do not run standalone unless debugging.

SKILL_NAME="${SKILL_NAME:-agnes-aigc-gen}"

# Default unified skill root (agent-agnostic)
DEFAULT_AGENTS_SKILL_ROOT="${DEFAULT_AGENTS_SKILL_ROOT:-$HOME/.agents/skills}"

# Known agent id -> skills parent directory (under $HOME)
skill_agent_parent_dir() {
  local agent
  agent="$(echo "$1" | tr '[:upper:]' '[:lower:]')"
  case "$agent" in
    agents) echo "$HOME/.agents/skills" ;;
    cursor) echo "$HOME/.cursor/skills" ;;
    claude) echo "$HOME/.claude/skills" ;;
    codex) echo "$HOME/.codex/skills" ;;
    openclaw) echo "$HOME/.openclaw/skills" ;;
    hermes) echo "$HOME/.hermes/skills" ;;
    *)
      echo "unknown agent: $agent (supported: agents,cursor,claude,codex,openclaw,hermes,all)" >&2
      return 1
      ;;
  esac
}

# Resolve INSTALL_AGENTS into a list of unique parent dirs (excluding default if duplicate)
skill_resolve_target_dirs() {
  local -a dirs=()
  local agents_spec="${INSTALL_AGENTS:-}"
  local agent parent

  if [[ -n "${INSTALL_SKILL_DIR:-}" ]]; then
    echo "$INSTALL_SKILL_DIR"
    return 0
  fi

  dirs+=("$DEFAULT_AGENTS_SKILL_ROOT")

  if [[ -z "$agents_spec" ]]; then
    printf '%s\n' "${dirs[@]}"
    return 0
  fi

  agents_spec="$(echo "$agents_spec" | tr '[:upper:]' '[:lower:]')"
  if [[ "$agents_spec" == "all" ]]; then
    agents_spec="cursor,claude,codex,openclaw,hermes"
  fi

  IFS=',' read -ra agent_list <<< "$agents_spec"
  for agent in "${agent_list[@]}"; do
    agent="$(echo "$agent" | tr -d '[:space:]')"
    [[ -z "$agent" || "$agent" == "agents" ]] && continue
    parent="$(skill_agent_parent_dir "$agent")" || return 1
    dirs+=("$parent")
  done

  # dedupe (bash 3.2 compatible)
  printf '%s\n' "${dirs[@]}" | awk '!seen[$0]++'
}

skill_setup_src() {
  if [[ -n "${SETUP_SRC:-}" && -f "$SETUP_SRC" ]]; then
    echo "$SETUP_SRC"
    return 0
  fi
  if [[ -n "${SKILL_REPO_ROOT:-}" && -f "$SKILL_REPO_ROOT/docs/SETUP.md" ]]; then
    echo "$SKILL_REPO_ROOT/docs/SETUP.md"
    return 0
  fi
  return 1
}

skill_install_from_local() {
  local src_dir="$1"
  local parent dest setup_src
  if [[ ! -d "$src_dir" || ! -f "$src_dir/SKILL.md" ]]; then
    echo "warning: skill source not found at $src_dir; skipping skill install" >&2
    return 0
  fi
  setup_src="$(skill_setup_src || true)"
  while IFS= read -r parent; do
    dest="$parent/$SKILL_NAME"
    echo "==> Installing skill to $dest"
    mkdir -p "$dest"
    rm -rf "$dest"
    mkdir -p "$dest"
    cp "$src_dir/SKILL.md" "$dest/SKILL.md"
    if [[ -n "$setup_src" ]]; then
      cp "$setup_src" "$dest/SETUP.md"
    fi
  done < <(skill_resolve_target_dirs)
}

skill_install_from_remote() {
  local repo="$1"
  local tag="$2"
  local parent dest file
  while IFS= read -r parent; do
    dest="$parent/$SKILL_NAME"
    echo "==> Installing skill to $dest"
    mkdir -p "$dest"
    curl -fsSL "https://raw.githubusercontent.com/${repo}/${tag}/skills/${SKILL_NAME}/SKILL.md" \
      -o "$dest/SKILL.md"
    curl -fsSL "https://raw.githubusercontent.com/${repo}/${tag}/docs/SETUP.md" \
      -o "$dest/SETUP.md"
  done < <(skill_resolve_target_dirs)
}

skill_install_summary() {
  echo "Skill install targets:"
  local parent
  while IFS= read -r parent; do
    echo "  $parent/$SKILL_NAME/SKILL.md"
  done < <(skill_resolve_target_dirs)
  echo ""
  echo "Optional: install to more agents, e.g.:"
  echo "  INSTALL_AGENTS=cursor,claude,codex,openclaw,hermes ./install.sh"
  echo "  INSTALL_AGENTS=all ./install-remote.sh"
}
