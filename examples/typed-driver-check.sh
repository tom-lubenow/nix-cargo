#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_TRIPLE="${NIX_CARGO_TARGET_TRIPLE:-x86_64-unknown-linux-gnu}"
TARGET_NAME="${NIX_CARGO_DRIVER_TARGET:-app}"

STATUS="$(
  nix eval --impure --raw --expr "
let
  flake = builtins.getFlake ${ROOT_DIR};
  drv = flake.legacyPackages.\${builtins.currentSystem}.mkDriver {
    src = ${ROOT_DIR}/examples/target-layout-workspace;
    targetTriple = \"${TARGET_TRIPLE}\";
    target = \"${TARGET_NAME}\";
  };
in
if (drv.passthru.targetSelection == \"${TARGET_NAME}\")
   && (drv.passthru.targetTriple == \"${TARGET_TRIPLE}\")
then \"ok\"
else \"bad\"
"
)"

if [ "${STATUS}" != "ok" ]; then
  echo "typed-driver-check: failed mkDriver typed assertions" >&2
  exit 1
fi

echo "typed-driver-check: ok"
