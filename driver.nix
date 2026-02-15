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
    nativeBuildInputs = [ pkgs.nix pkgs.rustc pkgs.cargo pkgs.stdenv.cc nixCargo ];

    PLAN_PKGS_PATH = toString pkgs.path;
    PLAN_SRC = toString srcStore;
    PLAN_CARGO_HOME = if cargoHome == null then "" else toString cargoHome;
    PLAN_GIT_SOURCE_HASHES = builtins.toJSON gitSourceHashes;
    PLAN_ALLOW_IMPURE_GIT_FETCH = if allowImpureGitFetch then "true" else "false";
    PLAN_MANIFEST = manifestStorePath;
    PLAN_RELEASE = if release then "true" else "false";
    PLAN_TARGET_TRIPLE = if targetTriple == null then "" else targetTriple;
    PLAN_TARGET = target;

    NIX_CONFIG = "extra-experimental-features = nix-command flakes ca-derivations dynamic-derivations recursive-nix";
  } ''
    set -euo pipefail

    planNix="$TMPDIR/nix-cargo-plan.nix"
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
      --output "$planNix"

    resolveNix="$TMPDIR/nix-cargo-resolve.nix"
    cat > "$resolveNix" <<'EOF'
let
  pkgs = import (builtins.getEnv "PLAN_PKGS_PATH") { };
  cargoHomePath = builtins.getEnv "PLAN_CARGO_HOME";
  plan = import (builtins.getEnv "PLAN_NIX") ({
    inherit pkgs;
    src = builtins.storePath (builtins.getEnv "PLAN_SRC");
    gitSourceHashes = builtins.fromJSON (builtins.getEnv "PLAN_GIT_SOURCE_HASHES");
    allowImpureGitFetch = (builtins.getEnv "PLAN_ALLOW_IMPURE_GIT_FETCH") == "true";
    release = (builtins.getEnv "PLAN_RELEASE") == "true";
  } // (
    if cargoHomePath == "" then { }
    else { cargoHome = builtins.storePath cargoHomePath; }
  ));
  targetKey = builtins.getEnv "PLAN_TARGET";
  packageKeys = builtins.attrNames plan.packageDerivations;
  nameMatches = builtins.filter (key: pkgs.lib.hasPrefix (targetKey + " v") key) packageKeys;
  matchedKey =
    if builtins.length nameMatches == 1 then builtins.head nameMatches
    else if builtins.length nameMatches == 0 then null
    else throw "nix-cargo-driver: target ''${targetKey}' is ambiguous; pass full package key";
  drvPath =
    if targetKey == "default" then
      plan.default.drvPath
    else if builtins.hasAttr targetKey plan.packageDerivations then
      (builtins.getAttr targetKey plan.packageDerivations).drvPath
    else if matchedKey != null then
      (builtins.getAttr matchedKey plan.packageDerivations).drvPath
    else
      throw "nix-cargo-driver: unknown target ''${targetKey}'";
in drvPath
EOF

    export PLAN_NIX="$planNix"
    drvPath="$(nix eval --raw --file "$resolveNix")"
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
      target =
        builtins.outputOf
          (builtins.unsafeDiscardOutputDependency (builtins.storePath plannedDrvPath))
          "out";
    };
})
