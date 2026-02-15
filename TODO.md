# nix-cargo TODO

## Priority 0 (correctness blockers)

- [x] Make planner failure strict by default (no partial captured-unit plans on Cargo compile error).
- [ ] Replace heuristic build-script replay with explicit, deterministic mapping from compile unit context to build-script binary execution.
  - [x] Added per-`runDir` build-script binary mapping to reduce global/fallback coupling.
  - [x] Replay control now uses explicit unit metadata (`custom-build` compile units).
  - [x] Remove remaining fallback heuristics.
  - [ ] Remove binary-path discovery fallback (`--out-dir` scan) by carrying resolved output path directly from planning.

## Priority 1 (correctness hardening)

- [x] Make build-script binary selection deterministic (no unordered `find | head` behavior).
- [x] Stop suppressing hydration copy errors (`cp ... || true`), or gate suppression behind explicit diagnostics.
- [x] Tighten path-marker rewriting to path-aware substitutions (avoid broad substring rewrites).

## Priority 2 (maintainability)

- [ ] Continue splitting `nix_emit.rs` into typed data modeling + renderer modules.
  - [x] Extracted cargo-home emission section into `src/nix_cargo_home_emit.rs`.
- [ ] Add focused regression tests for:
  - [x] strict planner failure behavior
  - [x] multi-build-script workspaces
  - [ ] cross-target + host-target mixed layouts
  - [ ] marker-rewrite edge cases

## In-progress now

- [ ] Execute remaining Priority 0 + Priority 1 hardening.
