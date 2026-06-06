#!/usr/bin/env bash
# Install from local source (requires Rust toolchain).
# For prebuilt binaries, use install-remote.sh instead.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_NAME="agnes-aigc-gen"
INSTALL_BIN_DIR="${INSTALL_BIN_DIR:-$HOME/.local/bin}"
SKILL_SRC="$ROOT/skills/agnes-aigc-gen"
SETUP_SRC="$ROOT/docs/SETUP.md"
export SETUP_SRC

# shellcheck source=scripts/install-skill.sh
source "$ROOT/scripts/install-skill.sh"
# shellcheck source=scripts/write-install-state.sh
source "$ROOT/scripts/write-install-state.sh"

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

if [[ "${SKIP_SKILL:-0}" != "1" ]]; then
  skill_install_from_local "$SKILL_SRC"
  echo ""
  skill_install_summary
fi

PKG_VERSION="$(grep '^version' "$ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')"
write_install_state "$INSTALL_BIN_DIR/$BIN_NAME" "$PKG_VERSION"

echo ""
echo "Done."
echo "  Binary: $INSTALL_BIN_DIR/$BIN_NAME"
echo ""
echo "Next steps:"
echo "  $BIN_NAME config set api-key YOUR_API_KEY"
echo "  $BIN_NAME config show"
