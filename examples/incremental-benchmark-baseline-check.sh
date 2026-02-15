#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASELINE_DIR="${ROOT_DIR}/examples/benchmark-baselines"

ENGINE="both"
WARMUP=1
UPDATE=0

usage() {
  cat <<'EOF'
Usage: incremental-benchmark-baseline-check.sh [options]

Validate incremental benchmark derivation-count snapshots for the small and large fixtures.
By default, validates both nix-cargo and cargo2nix and runs one warmup pass.

Options:
  --engine <nix-cargo|cargo2nix|both>  Engine(s) to validate. Default: both
  --no-warmup                          Skip warmup pass.
  --update                             Update baseline snapshots (requires --engine both).
  --help                               Show this help text.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --engine)
      ENGINE="$2"
      shift 2
      ;;
    --no-warmup)
      WARMUP=0
      shift
      ;;
    --update)
      UPDATE=1
      shift
      ;;
    --help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 1
fi

if [ "${ENGINE}" != "nix-cargo" ] && [ "${ENGINE}" != "cargo2nix" ] && [ "${ENGINE}" != "both" ]; then
  echo "invalid --engine value: ${ENGINE}" >&2
  exit 1
fi

if [ "${UPDATE}" -eq 1 ] && [ "${ENGINE}" != "both" ]; then
  echo "--update currently requires --engine both" >&2
  exit 1
fi

if [ -z "${CARGO2NIX_SETUP_CMD:-}" ]; then
  export CARGO2NIX_SETUP_CMD='nix run github:cargo2nix/cargo2nix -- --stdout > Cargo.nix'
fi
if [ -z "${CARGO2NIX_BUILD_CMD:-}" ]; then
  export CARGO2NIX_BUILD_CMD='nix build --no-link --impure --expr "let c2n = builtins.getFlake \"github:cargo2nix/cargo2nix\"; pkgs = import c2n.inputs.nixpkgs { system = builtins.currentSystem; overlays = [ c2n.overlays.default ]; }; rustPkgs = pkgs.rustBuilder.makePackageSet { rustVersion = \"1.83.0\"; packageFun = import ./Cargo.nix; }; in rustPkgs.workspace.app {}"'
fi

normalize_engine_snapshot() {
  local engine="$1"
  jq -c --arg engine "${engine}" '
    map({ engine, phase, derivations })
    | if $engine == "both" then . else map(select(.engine == $engine)) end
    | sort_by(.engine, .phase)
  '
}

run_small_snapshot() {
  "${ROOT_DIR}/examples/incremental-benchmark.sh" \
    --engine "${ENGINE}" \
    --workspace "${ROOT_DIR}/examples/incremental-workspace" \
    --target-crate app \
    --mutation-file "crates/corelib/src/lib.rs" \
    --json
}

run_large_snapshot() {
  "${ROOT_DIR}/examples/incremental-benchmark.sh" \
    --engine "${ENGINE}" \
    --workspace "${ROOT_DIR}/examples/incremental-workspace-large" \
    --target-crate app \
    --mutation-file "crates/leaf_a/src/lib.rs" \
    --json
}

validate_snapshot() {
  local label="$1"
  local expected_file="$2"
  local actual_json="$3"
  local expected_norm actual_norm

  expected_norm="$(normalize_engine_snapshot "${ENGINE}" < "${expected_file}")"
  actual_norm="$(printf '%s\n' "${actual_json}" | normalize_engine_snapshot "${ENGINE}")"

  if [ "${expected_norm}" != "${actual_norm}" ]; then
    echo "baseline mismatch: ${label}" >&2
    echo "expected: ${expected_norm}" >&2
    echo "actual:   ${actual_norm}" >&2
    return 1
  fi
}

if [ "${WARMUP}" -eq 1 ]; then
  run_small_snapshot >/dev/null
  run_large_snapshot >/dev/null
fi

small_actual="$(run_small_snapshot)"
large_actual="$(run_large_snapshot)"

if [ "${UPDATE}" -eq 1 ]; then
  printf '%s\n' "${small_actual}" | normalize_engine_snapshot "both" | jq '.' > "${BASELINE_DIR}/incremental-small.json"
  printf '%s\n' "${large_actual}" | normalize_engine_snapshot "both" | jq '.' > "${BASELINE_DIR}/incremental-large.json"
  echo "updated benchmark baseline snapshots"
  exit 0
fi

validate_snapshot "small" "${BASELINE_DIR}/incremental-small.json" "${small_actual}"
validate_snapshot "large" "${BASELINE_DIR}/incremental-large.json" "${large_actual}"

echo "benchmark-baseline-check: ok"
