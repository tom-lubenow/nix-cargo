#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE_DIR="${ROOT_DIR}/examples/failing-workspace"
PLAN_FILE="${WORKSPACE_DIR}/nix-cargo-plan.json"

if [ -z "${NIX_CARGO_BIN:-}" ]; then
  if [ -x "${ROOT_DIR}/target/debug/nix-cargo" ]; then
    NIX_CARGO_BIN="${ROOT_DIR}/target/debug/nix-cargo"
  else
    NIX_CARGO_BIN="nix-cargo"
  fi
fi

if "${NIX_CARGO_BIN}" plan --manifest-path "${WORKSPACE_DIR}/Cargo.toml" --json > /dev/null 2>&1; then
  echo "strict-planner-failure-check: expected plan failure for invalid workspace" >&2
  exit 1
fi

if "${NIX_CARGO_BIN}" emit --manifest-path "${WORKSPACE_DIR}/Cargo.toml" --output "${PLAN_FILE}" > /dev/null 2>&1; then
  echo "strict-planner-failure-check: expected emit failure for invalid workspace" >&2
  exit 1
fi

echo "strict-planner-failure-check: ok"
