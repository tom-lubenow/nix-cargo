# nix-cargo TODO

## Priority 0 (correctness)

- [x] Strict planner failure behavior (no partial plans on Cargo errors).
- [x] Deterministic build-script replay mapping using explicit unit metadata.
- [x] Direct backend cutover to store derivation materialization (`nix derivation add`) instead of text `.nix` emission.
- [x] Stable source snapshot naming for incremental rebuild reuse.

## Priority 1 (validation + quality)

- [x] Functional fixture coverage (`check-all.sh`) for proc-macro, build.rs, target layouts, typed driver wiring.
- [x] Incremental baseline checks (`incremental-benchmark-baseline-check.sh`, matrix baseline check).
- [ ] Add focused regression for output-path canonicalization handling in `nix-tool::derivation_add` retry path.
- [ ] Add focused regression for dependency artifact hydration from resolved output paths.

## Priority 2 (maintainability)

- [ ] Split `src/libstore_backend.rs` into smaller modules (`toolchain`, `source_stage`, `derivation_materialize`, `script_render`).
- [ ] Move JSON derivation-show parsing into `nix-tool` (typed helper API) so backend stops hand-parsing `serde_json::Value`.
- [ ] Add lightweight docs for backend data flow (`Plan` -> `MaterializedGraph` -> build target).

## In progress

- [ ] Add benchmark trend reporting (delta vs checked-in baselines) for CI output.
