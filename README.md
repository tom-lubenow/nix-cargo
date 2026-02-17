# nix-cargo

Prototype for Rust crate-level incremental builds in Nix using Cargo internals plus direct store-derivation materialization.

## What this does now

`nix-cargo` captures Cargo executor invocations (using Cargo library APIs), converts package units into per-package Nix derivations, and adds those derivations directly to the store via `nix derivation add`.

Commands:

- `graph`: print workspace package graph
- `plan`: print captured Cargo unit graph + rustc/build-script command metadata
- `emit`: materialize derivations and write a JSON manifest (package keys, drv paths, installables, layout metadata)
- `build`: materialize derivations and build a selected target package

The backend is now direct-derivation (`nix-libstore` + `nix-tool`), not text `.nix` emission.

## Quick usage

```bash
# inspect graph
cargo run -- graph --manifest-path /path/to/workspace/Cargo.toml --json

# inspect captured Cargo plan
cargo run -- plan --manifest-path /path/to/workspace/Cargo.toml --json

# materialize derivations and write manifest JSON
cargo run -- emit \
  --manifest-path /path/to/workspace/Cargo.toml \
  --output ./nix-cargo-plan.json

# build a selected target (default | full package key | unique crate name)
cargo run -- build \
  --manifest-path /path/to/workspace/Cargo.toml \
  --target default
```

## Driver integration

`driver.nix` and `flake.nix` expose typed driver entrypoints (`mkDriver`) that run planning/materialization as a build action and provide typed passthru target metadata.

## End-to-end checks

```bash
# core functional suite
./examples/check-all.sh

# incremental benchmark baseline checks
./examples/incremental-benchmark-baseline-check.sh --engine nix-cargo --no-warmup
./examples/incremental-benchmark-matrix-baseline-check.sh --engine nix-cargo --no-warmup
```

## Current behavior notes

- Planning is based on Cargo unstable internals (`compile_with_exec` + custom executor).
- Build replay is package-scoped and replays captured argv/env command shapes.
- Dependency hydration copies crate artifacts (`deps`, `build`, `examples`, `.fingerprint`) from dependency outputs into package-local target layout before replay.
- Build-script/proc-macro and host-vs-target layout are handled using captured unit metadata (`targetTriples`, `needsHostArtifacts`).
- Source snapshots are added to store with deterministic names to preserve derivation reuse across rebuilds.

## Fixtures

- `examples/incremental-workspace`
- `examples/proc-macro-workspace`
- `examples/target-layout-workspace`
- `examples/multi-build-script-workspace`
