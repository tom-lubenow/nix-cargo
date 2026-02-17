{ pkgs ? import <nixpkgs> {}
, src
, nixCargo
, manifestPath ? "${src}/Cargo.toml"
, cargoHome ? null
, gitSourceHashes ? { }
, allowImpureGitFetch ? false
, release ? false
, targetTriple ? null
, target ? "default"
, name ? "nix-cargo-driver"
}:

let
  srcStore = builtins.path { path = src; name = "nix-cargo-driver-src"; };
  srcPath = toString src;
  manifestPathStr = toString manifestPath;
  manifestStorePath =
    if manifestPathStr == "${srcPath}/Cargo.toml" then
      "${srcStore}/Cargo.toml"
    else if pkgs.lib.hasPrefix "${srcPath}/" manifestPathStr then
      "${srcStore}/${pkgs.lib.removePrefix "${srcPath}/" manifestPathStr}"
    else
      manifestPathStr;

  plannerDrv = pkgs.runCommand "${name}-ref" {
    __contentAddressed = true;
    outputHashMode = "text";
    outputHashAlgo = "sha256";

    requiredSystemFeatures = [ "recursive-nix" ];
    nativeBuildInputs = [ pkgs.nix pkgs.jq pkgs.rustc pkgs.cargo pkgs.stdenv.cc nixCargo ];

    PLAN_SRC = toString srcStore;
    PLAN_MANIFEST = manifestStorePath;
    PLAN_RELEASE = if release then "true" else "false";
    PLAN_TARGET_TRIPLE = if targetTriple == null then "" else targetTriple;
    PLAN_TARGET = target;

    NIX_CONFIG = "extra-experimental-features = nix-command flakes ca-derivations dynamic-derivations recursive-nix";
  } ''
    set -euo pipefail

    planJson="$TMPDIR/nix-cargo-plan.json"
    releaseArgs=()
    if [ "$PLAN_RELEASE" = "true" ]; then
      releaseArgs+=(--release)
    fi
    targetArgs=()
    if [ -n "$PLAN_TARGET_TRIPLE" ]; then
      targetArgs+=(--target-triple "$PLAN_TARGET_TRIPLE")
    fi
    export CARGO_TARGET_DIR="$TMPDIR/target"
    mkdir -p "$CARGO_TARGET_DIR"

    "${nixCargo}/bin/nix-cargo" emit \
      --manifest-path "$PLAN_MANIFEST" \
      "''${releaseArgs[@]}" \
      "''${targetArgs[@]}" \
      --output "$planJson"

    targetKey="$PLAN_TARGET"
    resolvedKey=""
    if [ "$targetKey" = "default" ]; then
      resolvedKey="$(jq -r '.default_workspace_package_key // empty' "$planJson")"
      if [ -z "$resolvedKey" ] || [ "$resolvedKey" = "null" ]; then
        echo "nix-cargo-driver: no default workspace target available" >&2
        exit 1
      fi
    elif jq -e --arg key "$targetKey" '.package_derivations | has($key)' "$planJson" >/dev/null; then
      resolvedKey="$targetKey"
    else
      mapfile -t nameMatches < <(jq -r --arg name "$targetKey" '. as $p | $p.workspace_package_keys[] | select($p.package_names[.] == $name)' "$planJson")
      if [ "''${#nameMatches[@]}" -eq 1 ]; then
        resolvedKey="''${nameMatches[0]}"
      elif [ "''${#nameMatches[@]}" -eq 0 ]; then
        echo "nix-cargo-driver: unknown target ''${targetKey}'" >&2
        exit 1
      else
        echo "nix-cargo-driver: target ''${targetKey}' is ambiguous; pass full package key" >&2
        exit 1
      fi
    fi

    drvPath="$(jq -r --arg key "$resolvedKey" '.package_derivations[$key] // empty' "$planJson")"
    if [ -z "$drvPath" ] || [ "$drvPath" = "null" ]; then
      echo "nix-cargo-driver: failed to resolve derivation path for target ''${resolvedKey}'" >&2
      exit 1
    fi
    printf '%s' "$drvPath" > "$out"
  '';
in
let
  plannedDrvPath = builtins.readFile plannerDrv;
in
plannerDrv.overrideAttrs (old: {
  passthru =
    (old.passthru or { })
    // {
      ref = plannerDrv;
      targetDrvPath = plannedDrvPath;
      targetSelection = target;
      targetTriple = targetTriple;
      target =
        builtins.outputOf
          (builtins.unsafeDiscardOutputDependency (builtins.storePath plannedDrvPath))
          "out";
    };
})
