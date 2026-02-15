#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

ENGINE="both"
WORKSPACE_DIR="${ROOT_DIR}/examples/incremental-workspace-large"
TARGET_CRATE="app"
JSON_OUTPUT=0
SCENARIOS_FILE=""

export CARGO2NIX_SETUP_CMD="${CARGO2NIX_SETUP_CMD:-nix run github:cargo2nix/cargo2nix -- --stdout > Cargo.nix}"
export CARGO2NIX_BUILD_CMD="${CARGO2NIX_BUILD_CMD:-nix build --no-link --impure --expr \"let c2n = builtins.getFlake \\\"github:cargo2nix/cargo2nix\\\"; pkgs = import c2n.inputs.nixpkgs { system = builtins.currentSystem; overlays = [ c2n.overlays.default ]; }; rustPkgs = pkgs.rustBuilder.makePackageSet { rustVersion = \\\"1.83.0\\\"; packageFun = import ./Cargo.nix; }; in rustPkgs.workspace.app {}\"}"

usage() {
  cat <<'EOF'
Usage: incremental-benchmark-matrix.sh [options]

Run a matrix of incremental benchmark scenarios against one workspace.

Options:
  --engine <nix-cargo|cargo2nix|both>  Benchmark engine(s). Default: both
  --workspace <path>                   Workspace to benchmark. Default: incremental-workspace-large
  --target-crate <name>                Target workspace crate. Default: app
  --scenarios-file <path>              Scenario TSV file: "<name>\t<mutation-file>" per line
  --json                               Emit JSON
  --help                               Show this help text.

Default scenarios (large workspace):
  leaf_a_edit   crates/leaf_a/src/lib.rs
  mid_a_edit    crates/mid_a/src/lib.rs
  core_edit     crates/core/src/lib.rs
  util_edit     crates/util/src/lib.rs
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --engine)
      ENGINE="$2"
      shift 2
      ;;
    --workspace)
      WORKSPACE_DIR="$2"
      shift 2
      ;;
    --target-crate)
      TARGET_CRATE="$2"
      shift 2
      ;;
    --scenarios-file)
      SCENARIOS_FILE="$2"
      shift 2
      ;;
    --json)
      JSON_OUTPUT=1
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

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/nix-cargo-bench-matrix.XXXXXX")"
RESULTS_FILE="${TMP_DIR}/results.jsonl"
trap 'rm -rf "${TMP_DIR}"' EXIT

run_scenario() {
  local name="$1"
  local mutation_file="$2"
  local result_json
  result_json="$(
    "${ROOT_DIR}/examples/incremental-benchmark.sh" \
      --engine "${ENGINE}" \
      --workspace "${WORKSPACE_DIR}" \
      --target-crate "${TARGET_CRATE}" \
      --mutation-file "${mutation_file}" \
      --json
  )"
  printf '%s\n' "${result_json}" \
    | jq -c --arg scenario "${name}" --arg mutation "${mutation_file}" '.[] | . + { scenario: $scenario, mutation_file: $mutation }' \
    >> "${RESULTS_FILE}"
}

if [ -n "${SCENARIOS_FILE}" ]; then
  while IFS=$'\t' read -r name mutation_file; do
    if [ -z "${name}" ] || [ -z "${mutation_file}" ]; then
      continue
    fi
    run_scenario "${name}" "${mutation_file}"
  done < "${SCENARIOS_FILE}"
else
  run_scenario "leaf_a_edit" "crates/leaf_a/src/lib.rs"
  run_scenario "mid_a_edit" "crates/mid_a/src/lib.rs"
  run_scenario "core_edit" "crates/core/src/lib.rs"
  run_scenario "util_edit" "crates/util/src/lib.rs"
fi

if [ "${JSON_OUTPUT}" -eq 1 ]; then
  jq -s 'sort_by(.scenario, .engine, .phase)' "${RESULTS_FILE}"
else
  printf '%-12s %-12s %-8s %-12s %-10s %-32s\n' \
    "scenario" "engine" "phase" "derivations" "elapsed_ms" "mutation_file"
  jq -r 'sort_by(.scenario, .engine, .phase)[] | [.scenario, .engine, .phase, (.derivations|tostring), (.elapsed_ms|tostring), .mutation_file] | @tsv' "${RESULTS_FILE}" \
    | while IFS=$'\t' read -r scenario engine phase derivations elapsed_ms mutation_file; do
        printf '%-12s %-12s %-8s %-12s %-10s %-32s\n' \
          "${scenario}" "${engine}" "${phase}" "${derivations}" "${elapsed_ms}" "${mutation_file}"
      done
fi
