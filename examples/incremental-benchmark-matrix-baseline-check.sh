#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

ENGINE="both"
WARMUP=1
UPDATE=0
SCENARIOS_FILE="${ROOT_DIR}/examples/benchmark-matrix-scenarios/repo.tsv"
BASELINE_FILE="${ROOT_DIR}/examples/benchmark-baselines/matrix-repo.json"

usage() {
  cat <<'EOF'
Usage: incremental-benchmark-matrix-baseline-check.sh [options]

Validate multi-scenario benchmark matrix derivation-count snapshots.

Options:
  --engine <nix-cargo|cargo2nix|both>  Engine(s) to validate. Default: both
  --scenarios-file <path>              Scenario TSV file.
  --baseline-file <path>               Baseline snapshot JSON.
  --no-warmup                          Skip warmup pass.
  --update                             Update baseline snapshot (requires --engine both).
  --help                               Show this help text.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --engine)
      ENGINE="$2"
      shift 2
      ;;
    --scenarios-file)
      SCENARIOS_FILE="$2"
      shift 2
      ;;
    --baseline-file)
      BASELINE_FILE="$2"
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

if [ ! -f "${SCENARIOS_FILE}" ]; then
  echo "scenario file not found: ${SCENARIOS_FILE}" >&2
  exit 1
fi

if [ -z "${CARGO2NIX_SETUP_CMD:-}" ]; then
  export CARGO2NIX_SETUP_CMD='nix run github:cargo2nix/cargo2nix -- --stdout > Cargo.nix'
fi
if [ -z "${CARGO2NIX_BUILD_CMD:-}" ]; then
  export CARGO2NIX_BUILD_CMD='nix build --no-link --impure --expr "let c2n = builtins.getFlake \"github:cargo2nix/cargo2nix\"; pkgs = import c2n.inputs.nixpkgs { system = builtins.currentSystem; overlays = [ c2n.overlays.default ]; }; rustPkgs = pkgs.rustBuilder.makePackageSet { rustVersion = \"1.83.0\"; packageFun = import ./Cargo.nix; }; in rustPkgs.workspace.app {}"'
fi

normalize_snapshot() {
  local engine="$1"
  jq -c --arg engine "${engine}" '
    map({
      scenario,
      workspace,
      target_crate,
      mutation_file,
      engine,
      phase,
      derivations
    })
    | if $engine == "both" then . else map(select(.engine == $engine)) end
    | sort_by(.scenario, .engine, .phase, .workspace, .target_crate, .mutation_file)
  '
}

run_matrix() {
  "${ROOT_DIR}/examples/incremental-benchmark-matrix.sh" \
    --engine "${ENGINE}" \
    --scenarios-file "${SCENARIOS_FILE}" \
    --json
}

if [ "${WARMUP}" -eq 1 ]; then
  run_matrix >/dev/null
fi

actual_json="$(run_matrix)"

if [ "${UPDATE}" -eq 1 ]; then
  printf '%s\n' "${actual_json}" \
    | normalize_snapshot "both" \
    | jq '.' > "${BASELINE_FILE}"
  echo "updated matrix baseline snapshot: ${BASELINE_FILE}"
  exit 0
fi

if [ ! -f "${BASELINE_FILE}" ]; then
  echo "baseline file not found: ${BASELINE_FILE}" >&2
  exit 1
fi

expected_norm="$(normalize_snapshot "${ENGINE}" < "${BASELINE_FILE}")"
actual_norm="$(printf '%s\n' "${actual_json}" | normalize_snapshot "${ENGINE}")"

if [ "${expected_norm}" != "${actual_norm}" ]; then
  echo "matrix baseline mismatch" >&2
  echo "expected: ${expected_norm}" >&2
  echo "actual:   ${actual_norm}" >&2
  exit 1
fi

echo "benchmark-matrix-baseline-check: ok"
