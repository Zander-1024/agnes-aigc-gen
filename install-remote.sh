#!/usr/bin/env bash
# Install agnes-aigc-gen from GitHub Releases (default).
# One-liner:
#   curl -fsSL https://raw.githubusercontent.com/Zander-1024/agnes-aigc-gen/master/install-remote.sh | bash
set -euo pipefail

REPO="${AGNES_AIGC_REPO:-Zander-1024/agnes-aigc-gen}"
BIN_NAME="agnes-aigc-gen"
INSTALL_BIN_DIR="${INSTALL_BIN_DIR:-$HOME/.local/bin}"

detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "${os}-${arch}" in
    Linux-x86_64) echo "linux-x86_64" ;;
    Linux-aarch64 | Linux-arm64) echo "linux-aarch64" ;;
    Darwin-x86_64) echo "darwin-x86_64" ;;
    Darwin-arm64) echo "darwin-aarch64" ;;
    *) echo "unsupported platform: ${os}-${arch}" >&2; exit 1 ;;
  esac
}

resolve_version() {
  if [[ -n "${AGNES_AIGC_VERSION:-}" ]]; then
    local v="${AGNES_AIGC_VERSION#v}"
    echo "$v"
    return
  fi
  curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\([^"]*\)".*/\1/p' \
    | head -1
}

install_skill() {
  if [[ "${SKIP_SKILL:-0}" == "1" ]]; then
    return
  fi
  local tag="v${VERSION}"
  local tmp_script
  tmp_script="$(mktemp)"
  curl -fsSL "https://raw.githubusercontent.com/${REPO}/${tag}/scripts/install-skill.sh" -o "$tmp_script"
  # shellcheck source=/dev/null
  source "$tmp_script"
  skill_install_from_remote "$REPO" "$tag"
  rm -f "$tmp_script"
  echo ""
  skill_install_summary
}

write_install_state_file() {
  local tag="v${VERSION}"
  local tmp_script
  tmp_script="$(mktemp)"
  curl -fsSL "https://raw.githubusercontent.com/${REPO}/${tag}/scripts/write-install-state.sh" -o "$tmp_script"
  # shellcheck source=/dev/null
  source "$tmp_script"
  write_install_state "$INSTALL_BIN_DIR/$BIN_NAME" "$VERSION"
  rm -f "$tmp_script"
}

PLATFORM="$(detect_platform)"
VERSION="$(resolve_version)"
TAG="v${VERSION}"
ARCHIVE="${BIN_NAME}-${VERSION}-${PLATFORM}.tar.gz"
BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "==> Installing ${BIN_NAME} ${TAG} (${PLATFORM})"
mkdir -p "$INSTALL_BIN_DIR"

curl -fsSL "${BASE_URL}/${ARCHIVE}" -o "$TMP/${ARCHIVE}"
if curl -fsSL "${BASE_URL}/SHA256SUMS.txt" -o "$TMP/SHA256SUMS.txt" 2>/dev/null; then
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$TMP" && grep " ${ARCHIVE}\$" SHA256SUMS.txt | sha256sum -c -)
  elif command -v shasum >/dev/null 2>&1; then
    (cd "$TMP" && grep " ${ARCHIVE}\$" SHA256SUMS.txt | shasum -a 256 -c -)
  fi
fi

tar -xzf "$TMP/${ARCHIVE}" -C "$TMP"
install -m 755 "$TMP/${BIN_NAME}" "$INSTALL_BIN_DIR/${BIN_NAME}"

install_skill
write_install_state_file

if [[ ":$PATH:" != *":$INSTALL_BIN_DIR:"* ]]; then
  echo ""
  echo "Note: add $INSTALL_BIN_DIR to your PATH, e.g.:"
  echo '  export PATH="$HOME/.local/bin:$PATH"'
  echo ""
fi

echo ""
echo "Done."
echo "  Binary: $INSTALL_BIN_DIR/$BIN_NAME"
echo "  Version: $TAG"
echo ""
echo "Next steps:"
echo "  $BIN_NAME config set api-key YOUR_API_KEY"
echo "  $BIN_NAME config show"
