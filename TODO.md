# nix-cargo TODO

## Priority 0 (correctness blockers)

- [x] Make planner failure strict by default (no partial captured-unit plans on Cargo compile error).
- [x] Replace heuristic build-script replay with explicit, deterministic mapping from compile unit context to build-script binary execution.
  - [x] Added per-`runDir` build-script binary mapping to reduce global/fallback coupling.
  - [x] Replay control now uses explicit unit metadata (`custom-build` compile units).
  - [x] Remove remaining fallback heuristics.
  - [x] Carry resolved build-script binary path from planning metadata into replay commands.
  - [x] Remove binary-path discovery fallback (`--out-dir` scan); fail fast when metadata is missing.

## Priority 1 (correctness hardening)

- [x] Make build-script binary selection deterministic (no unordered `find | head` behavior).
- [x] Stop suppressing hydration copy errors (`cp ... || true`), or gate suppression behind explicit diagnostics.
- [x] Tighten path-marker rewriting to path-aware substitutions (avoid broad substring rewrites).

## Priority 2 (maintainability)

- [x] Continue splitting `nix_emit.rs` into typed data modeling + renderer modules.
  - [x] Extracted cargo-home emission section into `src/nix_cargo_home_emit.rs`.
  - [x] Extracted preamble/header emission into `src/nix_header_emit.rs`.
  - [x] Extracted public-attrset emission section into `src/nix_public_attrs_emit.rs`.
  - [x] Extracted crate-plan emission section into `src/nix_crate_plan_emit.rs`.
  - [x] Extracted package-derivation emission section into `src/nix_package_derivation_emit.rs`.
  - [x] Added typed crate-plan render model in `src/nix_emit_model.rs`.
- [x] Add focused regression tests for:
  - [x] strict planner failure behavior
  - [x] multi-build-script workspaces
  - [x] cross-target + host-target mixed layouts
  - [x] marker-rewrite edge cases

## In-progress now

- [ ] Add a cargo2nix-enabled benchmark lane for environments that can afford tool bootstrap.

## Benchmarking

- [x] Add an incrementalism benchmark harness (`examples/incremental-benchmark.sh`) with
  cold/rebuild derivation-count and elapsed-time reporting.
- [x] Add CI-friendly baseline snapshots for benchmark fixtures.
- [x] Add a real-world benchmark matrix harness and baseline snapshots.
  - [x] Added local multi-scenario harness (`examples/incremental-benchmark-matrix.sh`).
  - [x] Added scenario-file format and reusable scenario set (`examples/benchmark-matrix-scenarios/large.tsv`).
  - [x] Added repo-backed scenario set (`examples/benchmark-matrix-scenarios/repo.tsv`).
  - [x] Added matrix baseline snapshot + checker (`examples/benchmark-baselines/matrix-repo.json`, `examples/incremental-benchmark-matrix-baseline-check.sh`).
- [x] Wire benchmark baseline checks into flake/CI checks.
  - [x] Added flake apps (`benchmark-baseline-check`, `benchmark-matrix-baseline-check`, `benchmark-ci-checks`).
  - [x] Added flake schema checks for baseline snapshots and scenario files.
