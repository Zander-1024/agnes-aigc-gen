#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_NAME="agnes-aigc-gen"
INSTALL_BIN_DIR="${INSTALL_BIN_DIR:-$HOME/.local/bin}"
INSTALL_SKILL_DIR="${INSTALL_SKILL_DIR:-$HOME/.cursor/skills}"
SKILL_SRC="$ROOT/skills/agnes-aigc-gen"
SKILL_DEST="$INSTALL_SKILL_DIR/agnes-aigc-gen"

echo "==> Building release binary..."
cd "$ROOT"
cargo build --release

RELEASE_BIN="$ROOT/target/release/$BIN_NAME"
if [[ ! -f "$RELEASE_BIN" ]]; then
  echo "error: binary not found at $RELEASE_BIN" >&2
  exit 1
fi

echo "==> Installing binary to $INSTALL_BIN_DIR/$BIN_NAME"
mkdir -p "$INSTALL_BIN_DIR"
install -m 755 "$RELEASE_BIN" "$INSTALL_BIN_DIR/$BIN_NAME"

if [[ ":$PATH:" != *":$INSTALL_BIN_DIR:"* ]]; then
  echo ""
  echo "Note: add $INSTALL_BIN_DIR to your PATH, e.g.:"
  echo '  export PATH="$HOME/.local/bin:$PATH"'
  echo ""
fi

if [[ -d "$SKILL_SRC" && -f "$SKILL_SRC/SKILL.md" ]]; then
  echo "==> Installing Cursor skill to $SKILL_DEST"
  mkdir -p "$INSTALL_SKILL_DIR"
  rm -rf "$SKILL_DEST"
  cp -R "$SKILL_SRC" "$SKILL_DEST"
  echo "Skill installed. Cursor loads skills from ~/.cursor/skills/<name>/SKILL.md"
else
  echo "warning: skill source not found at $SKILL_SRC; skipping skill install" >&2
fi

echo ""
echo "Done."
echo "  Binary: $INSTALL_BIN_DIR/$BIN_NAME"
echo "  Skill:  $SKILL_DEST/SKILL.md"
echo ""
echo "Next steps:"
echo "  $BIN_NAME config set api-key YOUR_API_KEY"
echo "  $BIN_NAME config show"
