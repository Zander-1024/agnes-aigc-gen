#!/usr/bin/env bash
# Smoke test: image-to-image, text-to-video, image-to-video
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${AGNES_BIN:-$ROOT/target/release/agnes-aigc-gen}"
INPUT="${ROOT}/test-input.png"

if [[ ! -x "$BIN" ]]; then
  echo "Building release binary..."
  (cd "$ROOT" && cargo build --release)
fi

if ! "$BIN" config show 2>&1 | grep -q 'api_key.*configured'; then
  echo "ERROR: API key not configured. Run:"
  echo "  $BIN config set api-key YOUR_API_KEY"
  exit 1
fi

# Verify decrypt works (config show does not decrypt)
if ! "$BIN" image -p "ping" --ratio 1:1 --output-format plain 2>/dev/null | head -1 | grep -qE '^https?://'; then
  if ! "$BIN" image -p "ping" --ratio 1:1 2>&1 | grep -qv 'decryption failed'; then
    echo "ERROR: API key decrypt failed on this machine. Re-run:"
    echo "  $BIN config set api-key YOUR_API_KEY"
    exit 1
  fi
fi

if [[ ! -f "$INPUT" ]]; then
  python3 - <<'PY' "$INPUT"
from PIL import Image
import sys
Image.new("RGB", (512, 512), (70, 130, 180)).save(sys.argv[1])
PY
fi

echo "=== 1/3 Image-to-image ==="
I2I_JSON=$("$BIN" -v image \
  -p "Transform into a watercolor painting with soft pastel colors, preserve composition" \
  --ratio 1:1 \
  -i "$INPUT" 2>"${ROOT}/.smoke-i2i.log")
echo "$I2I_JSON"
ASSET=$(echo "$I2I_JSON" | python3 -c "import json,sys; print(json.load(sys.stdin).get('asset_uri',''))")
if [[ -z "$ASSET" ]]; then
  echo "FAIL: i2i missing asset_uri"
  exit 1
fi
echo "OK i2i asset_uri=$ASSET"

echo ""
echo "=== 2/3 Text-to-video (may take several minutes) ==="
T2V_JSON=$("$BIN" -v video \
  -p "A cinematic shot of ocean waves at sunset, gentle camera drift, warm golden light" \
  --ratio 16:9 \
  -d 3 \
  -f 24 2>"${ROOT}/.smoke-t2v.log")
echo "$T2V_JSON"
echo "$T2V_JSON" | python3 -c "import json,sys; d=json.load(sys.stdin); assert d.get('uri','').startswith('http'); print('OK t2v uri=', d['uri'][:80], '...')"

echo ""
echo "=== 3/3 Image-to-video (may take several minutes) ==="
I2V_JSON=$("$BIN" -v video \
  -p "Subtle natural motion, soft breathing, hair moving gently in breeze" \
  --ratio 1:1 \
  -d 3 \
  -i "$ASSET" 2>"${ROOT}/.smoke-i2v.log")
echo "$I2V_JSON"
echo "$I2V_JSON" | python3 -c "import json,sys; d=json.load(sys.stdin); assert d.get('uri','').startswith('http'); print('OK i2v uri=', d['uri'][:80], '...')"

echo ""
echo "All smoke tests passed."
