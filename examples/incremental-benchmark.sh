#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE_DIR="${ROOT_DIR}/examples/incremental-workspace"
TARGET_CRATE="app"
MUTATION_FILE="crates/corelib/src/lib.rs"
ENGINE="nix-cargo"
KEEP_TMP=0
JSON_OUTPUT=0

usage() {
  cat <<'EOF'
Usage: incremental-benchmark.sh [options]

Options:
  --engine <nix-cargo|cargo2nix|both>  Benchmark engine(s). Default: nix-cargo
  --workspace <path>                   Workspace to benchmark.
  --target-crate <name>                Workspace member crate to build. Default: app
  --mutation-file <relpath>            Relative file path to mutate between runs.
  --json                               Emit JSON report.
  --keep-tmp                           Keep temporary benchmark workspace.
  --help                               Show this help text.

Cargo2nix engine:
  - Requires CARGO2NIX_BUILD_CMD.
  - Optional CARGO2NIX_SETUP_CMD runs once before cold build.
  - Commands execute inside the temporary workspace copy.
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
    --mutation-file)
      MUTATION_FILE="$2"
      shift 2
      ;;
    --json)
      JSON_OUTPUT=1
      shift
      ;;
    --keep-tmp)
      KEEP_TMP=1
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

if [ ! -d "${WORKSPACE_DIR}" ]; then
  echo "workspace does not exist: ${WORKSPACE_DIR}" >&2
  exit 1
fi

if [ -z "${NIX_CARGO_BIN:-}" ]; then
  if [ -x "${ROOT_DIR}/target/debug/nix-cargo" ]; then
    NIX_CARGO_BIN="${ROOT_DIR}/target/debug/nix-cargo"
  else
    NIX_CARGO_BIN="nix-cargo"
  fi
fi

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/nix-cargo-bench.XXXXXX")"
REPORT_FILE="${TMP_DIR}/report.tsv"

cleanup() {
  if [ "${KEEP_TMP}" -eq 0 ]; then
    rm -rf "${TMP_DIR}"
  fi
}
trap cleanup EXIT

parse_derivation_count() {
  local log_file="$1"
  local count
  count="$(sed -nE 's/.*these ([0-9]+) derivations will be built.*/\1/p' "${log_file}" | tail -n1)"
  if [ -n "${count}" ]; then
    printf '%s' "${count}"
    return
  fi
  if grep -q 'this derivation will be built' "${log_file}"; then
    printf '1'
    return
  fi
  printf '0'
}

measure_phase() {
  local engine="$1"
  local phase="$2"
  shift 2

  local log_file="${TMP_DIR}/${engine}-${phase}.log"
  local start_ns end_ns elapsed_ms derivation_count
  start_ns="$(date +%s%N)"
  if ! "$@" >"${log_file}" 2>&1; then
    echo "benchmark phase failed: engine=${engine} phase=${phase}" >&2
    cat "${log_file}" >&2
    return 1
  fi
  end_ns="$(date +%s%N)"
  elapsed_ms="$(( (end_ns - start_ns) / 1000000 ))"
  derivation_count="$(parse_derivation_count "${log_file}")"
  printf '%s\t%s\t%s\t%s\n' "${engine}" "${phase}" "${derivation_count}" "${elapsed_ms}" >> "${REPORT_FILE}"
}

mutate_workspace() {
  local workspace_copy="$1"
  local mutation_path="${workspace_copy}/${MUTATION_FILE}"
  if [ ! -f "${mutation_path}" ]; then
    echo "mutation file not found: ${mutation_path}" >&2
    return 1
  fi
  printf '\n// incremental-benchmark mutation\n' >> "${mutation_path}"
}

run_nix_cargo_phase() {
  local workspace_copy="$1"
  local plan_json="${workspace_copy}/nix-cargo-bench-plan.json"
  local plan_nix="${workspace_copy}/nix-cargo-bench-plan.nix"

  "${NIX_CARGO_BIN}" plan --manifest-path "${workspace_copy}/Cargo.toml" --json > "${plan_json}"
  "${NIX_CARGO_BIN}" emit --manifest-path "${workspace_copy}/Cargo.toml" --output "${plan_nix}" > /dev/null

  local target_keys target_key_count target_key
  target_keys="$(
    jq -r --arg name "${TARGET_CRATE}" '
      [.packages[] | select(.workspace_member and .name == $name) | .key] | .[]
    ' "${plan_json}"
  )"
  target_key_count="$(printf '%s\n' "${target_keys}" | sed '/^$/d' | wc -l | tr -d ' ')"
  if [ "${target_key_count}" -ne 1 ]; then
    echo "expected exactly one workspace package named '${TARGET_CRATE}', found ${target_key_count}" >&2
    return 1
  fi
  target_key="$(printf '%s\n' "${target_keys}" | sed '/^$/d')"

  nix build --no-link --impure --expr "let p = import ${plan_nix} {}; in p.workspacePackages.\"${target_key}\""
}

run_cargo2nix_setup() {
  local workspace_copy="$1"
  if [ -z "${CARGO2NIX_SETUP_CMD:-}" ]; then
    return 0
  fi
  (cd "${workspace_copy}" && bash -lc "${CARGO2NIX_SETUP_CMD}")
}

run_cargo2nix_phase() {
  local workspace_copy="$1"
  if [ -z "${CARGO2NIX_BUILD_CMD:-}" ]; then
    echo "CARGO2NIX_BUILD_CMD is required for cargo2nix benchmark" >&2
    return 1
  fi
  (cd "${workspace_copy}" && bash -lc "${CARGO2NIX_BUILD_CMD}")
}

benchmark_nix_cargo() {
  local workspace_copy="${TMP_DIR}/nix-cargo-workspace"
  mkdir -p "${workspace_copy}"
  cp -R "${WORKSPACE_DIR}/." "${workspace_copy}/"
  measure_phase "nix-cargo" "cold" run_nix_cargo_phase "${workspace_copy}"
  mutate_workspace "${workspace_copy}"
  measure_phase "nix-cargo" "rebuild" run_nix_cargo_phase "${workspace_copy}"
}

benchmark_cargo2nix() {
  local workspace_copy="${TMP_DIR}/cargo2nix-workspace"
  mkdir -p "${workspace_copy}"
  cp -R "${WORKSPACE_DIR}/." "${workspace_copy}/"
  run_cargo2nix_setup "${workspace_copy}"
  measure_phase "cargo2nix" "cold" run_cargo2nix_phase "${workspace_copy}"
  mutate_workspace "${workspace_copy}"
  measure_phase "cargo2nix" "rebuild" run_cargo2nix_phase "${workspace_copy}"
}

case "${ENGINE}" in
  nix-cargo)
    benchmark_nix_cargo
    ;;
  cargo2nix)
    benchmark_cargo2nix
    ;;
  both)
    benchmark_nix_cargo
    if [ -z "${CARGO2NIX_BUILD_CMD:-}" ]; then
      echo "skipping cargo2nix benchmark in 'both' mode: CARGO2NIX_BUILD_CMD is unset" >&2
    else
      benchmark_cargo2nix
    fi
    ;;
  *)
    echo "invalid engine: ${ENGINE}" >&2
    exit 1
    ;;
esac

if [ "${JSON_OUTPUT}" -eq 1 ]; then
  awk -F '\t' '
    BEGIN { printf "[" }
    {
      if (NR > 1) printf ","
      printf "{\"engine\":\"%s\",\"phase\":\"%s\",\"derivations\":%d,\"elapsed_ms\":%d}",
        $1, $2, $3, $4
    }
    END { print "]" }
  ' "${REPORT_FILE}"
else
  printf '%-12s %-8s %-12s %-10s\n' "engine" "phase" "derivations" "elapsed_ms"
  while IFS=$'\t' read -r engine phase derivations elapsed_ms; do
    printf '%-12s %-8s %-12s %-10s\n' "${engine}" "${phase}" "${derivations}" "${elapsed_ms}"
  done < "${REPORT_FILE}"
  echo
  while IFS= read -r engine; do
    local_cold="$(awk -F '\t' -v e="${engine}" '$1 == e && $2 == "cold" { print $3 }' "${REPORT_FILE}")"
    local_rebuild="$(awk -F '\t' -v e="${engine}" '$1 == e && $2 == "rebuild" { print $3 }' "${REPORT_FILE}")"
    printf '%s: cold=%s rebuild=%s\n' "${engine}" "${local_cold:-0}" "${local_rebuild:-0}"
  done < <(cut -f1 "${REPORT_FILE}" | sort -u)
fi

if [ "${KEEP_TMP}" -eq 1 ]; then
  echo "benchmark tmp kept: ${TMP_DIR}" >&2
fi

