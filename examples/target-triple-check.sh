#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE_DIR="${ROOT_DIR}/examples/target-layout-workspace"
PLAN_FILE="${WORKSPACE_DIR}/nix-cargo-target-triple-plan.nix"
TARGET_TRIPLE="${NIX_CARGO_TARGET_TRIPLE:-x86_64-unknown-linux-gnu}"
APP_KEY="app v0.1.0 (${WORKSPACE_DIR}/crates/app)"

if [ -z "${NIX_CARGO_BIN:-}" ]; then
  if [ -x "${ROOT_DIR}/target/debug/nix-cargo" ]; then
    NIX_CARGO_BIN="${ROOT_DIR}/target/debug/nix-cargo"
  else
    NIX_CARGO_BIN="nix-cargo"
  fi
fi

"${NIX_CARGO_BIN}" emit \
  --manifest-path "${WORKSPACE_DIR}/Cargo.toml" \
  --target-triple "${TARGET_TRIPLE}" \
  --output "${PLAN_FILE}"

TARGET_STATUS="$(
  nix eval --impure --raw --expr "
let
  plan = import ${PLAN_FILE} {};
  app = plan.packageLayouts.\"${APP_KEY}\";
in
if (plan.targetTriple == \"${TARGET_TRIPLE}\")
   && (builtins.elem \"${TARGET_TRIPLE}\" app.targetTriples)
then \"ok\"
else \"bad\"
"
)"

if [ "${TARGET_STATUS}" != "ok" ]; then
  echo "target-triple-check: failed target-triple assertions" >&2
  exit 1
fi

echo "target-triple-check: ok"

