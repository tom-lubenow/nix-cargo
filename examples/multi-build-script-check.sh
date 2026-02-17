#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE_DIR="${ROOT_DIR}/examples/multi-build-script-workspace"

if [ -z "${NIX_CARGO_BIN:-}" ]; then
  if [ -x "${ROOT_DIR}/target/debug/nix-cargo" ]; then
    NIX_CARGO_BIN="${ROOT_DIR}/target/debug/nix-cargo"
  else
    NIX_CARGO_BIN="nix-cargo"
  fi
fi

APP_OUT="$("${NIX_CARGO_BIN}" build --manifest-path "${WORKSPACE_DIR}/Cargo.toml" --target app | tail -n1)"
APP_BIN="$(find "${APP_OUT}/deps" -maxdepth 1 -type f -name 'app-*' | head -n1)"
OUTPUT="$("${APP_BIN}")"

if [ "${OUTPUT}" != "from-a from-b" ]; then
  printf 'unexpected output: %s\n' "${OUTPUT}" >&2
  exit 1
fi

echo "multi-build-script-check: ok"
