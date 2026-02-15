#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE_DIR="${ROOT_DIR}/examples/target-layout-workspace"
PLAN_FILE="${WORKSPACE_DIR}/nix-cargo-plan.nix"
TARGET_TRIPLE="${NIX_CARGO_TARGET_TRIPLE:-x86_64-unknown-linux-gnu}"

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

APP_KEY="app v0.1.0 (${WORKSPACE_DIR}/crates/app)"
GEN_KEY="genmsg v0.1.0 (${WORKSPACE_DIR}/crates/genmsg)"

LAYOUT_STATUS="$(
  nix eval --impure --raw --expr "
let
  plan = import ${PLAN_FILE} {};
  app = plan.packageLayouts.\"${APP_KEY}\";
  gen = plan.packageLayouts.\"${GEN_KEY}\";
in
if (builtins.elem \"${TARGET_TRIPLE}\" app.targetTriples)
   && (!app.needsHostArtifacts)
   && (builtins.elem \"${TARGET_TRIPLE}\" gen.targetTriples)
   && gen.needsHostArtifacts
then \"ok\"
else \"bad\"
"
)"

if [ "${LAYOUT_STATUS}" != "ok" ]; then
  echo "target-layout-check: failed layout assertions" >&2
  exit 1
fi

echo "target-layout-check: ok"
