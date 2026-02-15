# nix-cargo

Prototype crate for exploring a Nix-native Cargo build graph emitter.

## What this MVP does

`nix-cargo` loads workspaces with Cargo library APIs and captures compiler invocations from Cargo's
internal compile pipeline.

- `graph` prints the workspace package graph (including workspace-only deps).
- `plan` emits a full resolved package DAG plus structured compile units (`cwd` + `env` +
  `program` + `args`) captured from Cargo's internal executor.
  Units also include `target_triple` metadata when Cargo emitted `--target`.
- `plan`/`emit` support `--target-triple <triple>` to plan cross-target command graphs through
  Cargo internals (`BuildConfig.requested_kinds`).
- `emit` emits a Nix expression with one derivation per resolved package (workspace + external),
  replaying each package's own captured Cargo-executor command sequence with deterministic path
  marker substitution.
  It also emits dynamic package refs (`builtins.outputOf`) per package.
- `emit` auto-materializes `cargoHome` from `Cargo.lock` for crates.io registry dependencies
  (checksum-verified fixed-output fetches) when `cargoHome = null`.
- `emit` auto-materializes git dependencies when `cargoHome = null` using `gitSourceHashes`
  (deterministic `pkgs.fetchgit`), with opt-in `allowImpureGitFetch = true` fallback.
- `driver.nix` runs planning as a build action and exposes a dynamic `passthru.target` resolved
  from the planned package/workspace `.drv` path.
- `flake.nix` exports `legacyPackages.<system>.mkDriver` to construct dynamic planner drivers for
  arbitrary Rust workspaces.
  That entrypoint is typed via `lib.evalModules` (`lib/mk-driver.nix`) so driver args are
  validated (paths, bools, optional fields, etc.).

## Quick end-to-end MVP

```bash
# 1) Inspect graph
cargo run -- graph --json

# 2) Emit nix expression for a workspace
cargo run -- emit --output ./nix-cargo-plan.nix \
  --manifest-path /path/to/workspace/Cargo.toml

# 2b) Emit for an explicit target triple
cargo run -- emit --output ./nix-cargo-plan-aarch64.nix \
  --manifest-path /path/to/workspace/Cargo.toml \
  --target-triple aarch64-unknown-linux-gnu

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
# Example typed mkDriver usage:
#   nix eval --raw --expr '
#     let
#       flake = builtins.getFlake (toString ./.);
#       drv = flake.legacyPackages.${builtins.currentSystem}.mkDriver {
#         src = ./examples/target-layout-workspace;
#         targetTriple = "x86_64-unknown-linux-gnu";
#         target = "app";
#       };
#     in drv.passthru.target
#   '
#
# Optional for git dependencies with auto cargoHome:
#   - deterministic: pass gitSourceHashes = { "<git+source>" = "sha256-..."; ...; }
#   - fallback: set allowImpureGitFetch = true
#
# Driver target selection:
#   - `target = "default"` builds workspace default output
#   - `target = "<full package key>"` exact package
#   - `target = "<crate-name>"` works if the crate name is unique in the resolved graph
# Driver planning target:
#   - `targetTriple = "aarch64-unknown-linux-gnu"` forwards to `nix-cargo emit --target-triple ...`
#   - passthru metadata includes `targetSelection` and `targetTriple`

# 6) Minimal workspace integration check
#    ./examples/integration-check.sh
#    # optionally:
#    NIX_CARGO_BIN=./target/debug/nix-cargo ./examples/integration-check.sh
#
# 7) Proc-macro + build.rs fixture check
#    ./examples/proc-macro-check.sh
#    # optionally:
#    NIX_CARGO_BIN=./target/debug/nix-cargo ./examples/proc-macro-check.sh
#
# 8) Run both checks
#    ./examples/check-all.sh
#
# 9) Target-layout host/target split check
#    ./examples/target-layout-check.sh
#    # optionally:
#    NIX_CARGO_BIN=./target/debug/nix-cargo ./examples/target-layout-check.sh
#    NIX_CARGO_TARGET_TRIPLE=x86_64-unknown-linux-gnu ./examples/target-layout-check.sh
#
# 10) Explicit target-triple propagation check
#     ./examples/target-triple-check.sh
#     # optionally:
#     NIX_CARGO_BIN=./target/debug/nix-cargo ./examples/target-triple-check.sh
#
# 11) Typed mkDriver wiring check
#     ./examples/typed-driver-check.sh
```

## Notes

- Planning uses unstable Cargo internals (`compile_with_exec` + custom `Executor`) and is
  intentionally tied to current Cargo internals.
- The current planner executes Cargo build jobs while recording Cargo executor commands to ensure
  complete command capture (including build-script-sensitive crates).
- Planner execution is serialized (`jobs = 1`) so captured command order stays deterministic for
  reproducible emitted Nix plans.
- The emitted expression accepts `cargoHome` (default `null`) and rewrites captured absolute paths
  via deterministic markers at replay time.
- With `cargoHome = null`, emitted Nix builds a deterministic cargo home from `Cargo.lock`
  checksums for crates.io registry packages.
- Git sources are auto-materialized with `gitSourceHashes`; if missing, you can either pass
  `cargoHome` or set `allowImpureGitFetch = true`.
- Repeated packages from the same git source share one fetch binding in emitted Nix (single fetch,
  multiple checkout copy destinations).
- Non-crates.io registry sources are not auto-materialized yet; for those, pass a pre-populated
  `cargoHome` override.
- `gitSourceHashes` keys are full Cargo source strings (e.g.
  `"git+https://github.com/org/repo?rev=<rev>#<commit>" = "sha256-...";`).
- Package derivations are keyed by resolved `PackageId` string, allowing multiple versions of the
  same crate in one graph.
- Dynamic refs are exposed as `dynamicPackages` and `workspaceDynamicPackages` (string outputs from
  `builtins.outputOf`).
- Layout metadata is exposed as `packageLayouts` and `workspacePackageLayouts`.
- Emitted plans expose the requested target as `targetTriple` (or `null` when unspecified).
- A driver-like surface is exposed at `driver` with `targets.<package-id>.target` and
  `workspaceTargets`.
- Current Nix emitter remains MVP quality: it seeds `CARGO_TARGET_DIR` from dependency outputs,
  then replays package-scoped captured calls as structured argv/env invocations.
- Included example workspace: `examples/incremental-workspace`.
- Included complex fixture workspace (`proc-macro` + `build.rs`):
  `examples/proc-macro-workspace`.
- Included target-layout fixture workspace (`build.target` + `build.rs`):
  `examples/target-layout-workspace`.
- Captured package derivations are emitted in explicit topological dependency order.
- Dependency output hydration copies `deps`, `build`, `.fingerprint`, and target-triple variants to
  support strict per-package replay for build-script and proc-macro heavy graphs.
- Host-vs-target replay is now explicit per package via captured command metadata
  (`targetTriples`/`needsHostArtifacts`) instead of broad directory globbing.
- Host-vs-target inference is unit-aware (`target_kind`/`compile_mode`), so `custom-build` and
  `proc-macro` units stay on host artifact layouts even in cross-target graphs.
- `Plan` JSON now includes `target_triple` when planning is performed with `--target-triple`.
