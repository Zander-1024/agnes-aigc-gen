#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

echo "=== cargo fmt --check ==="
cargo fmt --all -- --check

echo "=== cargo clippy ==="
cargo clippy --all-targets -- -D warnings

echo "=== cargo test ==="
cargo test

echo "=== cargo build --release ==="
cargo build --release

echo "=== CI local: OK ==="
