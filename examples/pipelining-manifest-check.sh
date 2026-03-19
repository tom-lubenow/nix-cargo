#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE_DIR="${ROOT_DIR}/examples/incremental-workspace-large"
PLAN_FILE="${WORKSPACE_DIR}/nix-cargo-phase-plan.json"

if [ -z "${NIX_CARGO_BIN:-}" ]; then
  if [ -x "${ROOT_DIR}/target/debug/nix-cargo" ]; then
    NIX_CARGO_BIN="${ROOT_DIR}/target/debug/nix-cargo"
  else
    NIX_CARGO_BIN="nix-cargo"
  fi
fi

"${NIX_CARGO_BIN}" emit --manifest-path "${WORKSPACE_DIR}/Cargo.toml" --output "${PLAN_FILE}"

MID_A_KEY="$(jq -r '. as $p | $p.workspace_package_keys[] | select($p.package_names[.] == "mid_a")' "${PLAN_FILE}")"
LEAF_A_KEY="$(jq -r '. as $p | $p.workspace_package_keys[] | select($p.package_names[.] == "leaf_a")' "${PLAN_FILE}")"
APP_KEY="$(jq -r '. as $p | $p.workspace_package_keys[] | select($p.package_names[.] == "app")' "${PLAN_FILE}")"

jq -e \
  --arg mid "$MID_A_KEY" \
  --arg leaf "$LEAF_A_KEY" \
  --arg app "$APP_KEY" \
  '
  (.package_phases[$mid].metadata.derivation | type) == "string"
  and (.package_phases[$mid].full.derivation | type) == "string"
  and (.package_phases[$mid].metadata.dependency_phases[$leaf] == "metadata")
  and (.package_phases[$mid].full.dependency_phases[$leaf] == "metadata")
  and (.package_phases[$app].metadata == null)
  ' \
  "${PLAN_FILE}" > /dev/null

echo "pipelining-manifest-check: ok"
