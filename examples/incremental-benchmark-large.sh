#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ -z "${CARGO2NIX_SETUP_CMD:-}" ]; then
  export CARGO2NIX_SETUP_CMD='nix run github:cargo2nix/cargo2nix -- --stdout > Cargo.nix'
fi
if [ -z "${CARGO2NIX_BUILD_CMD:-}" ]; then
  export CARGO2NIX_BUILD_CMD='nix build --no-link --impure --expr "let c2n = builtins.getFlake \"github:cargo2nix/cargo2nix\"; pkgs = import c2n.inputs.nixpkgs { system = builtins.currentSystem; overlays = [ c2n.overlays.default ]; }; rustPkgs = pkgs.rustBuilder.makePackageSet { rustVersion = \"1.83.0\"; packageFun = import ./Cargo.nix; }; in rustPkgs.workspace.app {}"'
fi

exec "${ROOT_DIR}/examples/incremental-benchmark.sh" \
  --engine both \
  --workspace "${ROOT_DIR}/examples/incremental-workspace-large" \
  --target-crate app \
  --mutation-file "crates/leaf_a/src/lib.rs" \
  "$@"
