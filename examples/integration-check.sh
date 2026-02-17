#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE_DIR="${ROOT_DIR}/examples/incremental-workspace"
PLAN_FILE="${ROOT_DIR}/examples/incremental-workspace/nix-cargo-plan.json"
if [ -z "${NIX_CARGO_BIN:-}" ]; then
  if [ -x "${ROOT_DIR}/target/debug/nix-cargo" ]; then
    NIX_CARGO_BIN="${ROOT_DIR}/target/debug/nix-cargo"
  else
    NIX_CARGO_BIN="nix-cargo"
  fi
fi

"${NIX_CARGO_BIN}" graph --manifest-path "${WORKSPACE_DIR}/Cargo.toml" --json > /dev/null
"${NIX_CARGO_BIN}" plan --manifest-path "${WORKSPACE_DIR}/Cargo.toml" --json > /dev/null
"${NIX_CARGO_BIN}" emit --manifest-path "${WORKSPACE_DIR}/Cargo.toml" --output "${PLAN_FILE}"

jq -e '.package_derivations | length > 0' "${PLAN_FILE}" > /dev/null

echo "integration-check: ok"
