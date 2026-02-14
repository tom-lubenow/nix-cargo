# nix-cargo

Prototype crate for exploring a Nix-native Cargo build graph emitter.

## What this MVP does

`nix-cargo` loads workspaces with Cargo library APIs and captures compiler invocations from Cargo's
internal compile pipeline.

- `graph` prints the workspace package graph (including workspace-only deps).
- `plan` emits a full resolved package DAG plus structured compile units (`cwd` + `env` +
  `program` + `args`) captured from Cargo's internal executor.
- `emit` emits a Nix expression with one derivation per resolved package (workspace + external),
  replaying each package's own captured Cargo-executor command sequence with deterministic path
  marker substitution.
  It also emits dynamic-derivation wrapper refs (`builtins.outputOf`) per package.
- `emit` auto-materializes `cargoHome` from `Cargo.lock` for crates.io registry dependencies
  (checksum-verified fixed-output fetches) when `cargoHome = null`.
- `emit` auto-materializes git dependencies when `cargoHome = null` using `gitSourceHashes`
  (deterministic `pkgs.fetchgit`), with opt-in `allowImpureGitFetch = true` fallback.
- `driver.nix` runs planning as a build action (`.drv` text output) and exposes
  `passthru.target = builtins.outputOf plannerDrv.outPath "out"` for RFC-92 dynamic usage.
- `flake.nix` exports `legacyPackages.<system>.mkDriver` to construct dynamic planner drivers for
  arbitrary Rust workspaces.

## Quick end-to-end MVP

```bash
# 1) Inspect graph
cargo run -- graph --json

# 2) Emit nix expression for a workspace
cargo run -- emit --output ./nix-cargo-plan.nix \
  --manifest-path /path/to/workspace/Cargo.toml

# 3) Evaluate expression (sanity)
nix-instantiate --eval ./nix-cargo-plan.nix

# 4) Use generated outputs (attrset):
#    .#packages."<package-id>", .#dynamicPackages."<package-id>", .#workspacePackages,
#    .#workspaceDynamicPackages, .#driver.targets."<package-id>".target, .#default,
#    .#packageDerivations

# 5) Build-time planner driver (dynamic derivation entrypoint)
#    nix build .#driver-default
#    nix eval --raw .#driver-default.passthru.target
#
# Optional for git dependencies with auto cargoHome:
#   - deterministic: pass gitSourceHashes = { "<git+source>" = "sha256-..."; ...; }
#   - fallback: set allowImpureGitFetch = true

# 6) Minimal workspace integration check
#    ./examples/integration-check.sh
#    # optionally:
#    NIX_CARGO_BIN=./target/debug/nix-cargo ./examples/integration-check.sh
```

## Notes

- Planning uses unstable Cargo internals (`compile_with_exec` + custom `Executor`) and is
  intentionally tied to current Cargo internals.
- The current planner executes Cargo build jobs while recording Cargo executor commands to ensure
  complete command capture (including build-script-sensitive crates).
- The emitted expression accepts `cargoHome` (default `null`) and rewrites captured absolute paths
  via deterministic markers at replay time.
- With `cargoHome = null`, emitted Nix builds a deterministic cargo home from `Cargo.lock`
  checksums for crates.io registry packages.
- Git sources are auto-materialized with `gitSourceHashes`; if missing, you can either pass
  `cargoHome` or set `allowImpureGitFetch = true`.
- Non-crates.io registry sources are not auto-materialized yet; for those, pass a pre-populated
  `cargoHome` override.
- `gitSourceHashes` keys are full Cargo source strings (e.g.
  `"git+https://github.com/org/repo?rev=<rev>#<commit>" = "sha256-...";`).
- Package derivations are keyed by resolved `PackageId` string, allowing multiple versions of the
  same crate in one graph.
- Dynamic refs are exposed as `dynamicPackages` and `workspaceDynamicPackages` (string outputs from
  `builtins.outputOf`).
- A driver-like surface is exposed at `driver` with `targets.<package-id>.target` and
  `workspaceTargets`.
- Current Nix emitter remains MVP quality: it seeds `CARGO_TARGET_DIR` from dependency outputs,
  then replays package-scoped captured calls as structured argv/env invocations.
- Included example workspace: `examples/incremental-workspace`.
