#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE_DIR="${ROOT_DIR}/examples/proc-macro-workspace"
PLAN_FILE="${WORKSPACE_DIR}/nix-cargo-plan.nix"

if [ -z "${NIX_CARGO_BIN:-}" ]; then
  if [ -x "${ROOT_DIR}/target/debug/nix-cargo" ]; then
    NIX_CARGO_BIN="${ROOT_DIR}/target/debug/nix-cargo"
  else
    NIX_CARGO_BIN="nix-cargo"
  fi
fi

"${NIX_CARGO_BIN}" emit --manifest-path "${WORKSPACE_DIR}/Cargo.toml" --output "${PLAN_FILE}"

APP_KEY="app v0.1.0 (${WORKSPACE_DIR}/crates/app)"
nix build --impure --expr "let p = import ${PLAN_FILE} {}; in p.workspacePackages.\"${APP_KEY}\"" >/dev/null

APP_BIN="$(find "${ROOT_DIR}/result/deps" -maxdepth 1 -type f -name 'app-*' | head -n1)"
OUTPUT="$("${APP_BIN}")"

if [ "${OUTPUT}" != "hello-from-build-script 42" ]; then
  printf 'unexpected output: %s\n' "${OUTPUT}" >&2
  exit 1
fi

echo "proc-macro-check: ok"

