{ lib
, pkgs
, nixCargo
, driverFile ? ../driver.nix
}:

let
  typedArgsModule = { config, ... }: {
    options = {
      src = lib.mkOption {
        type = lib.types.path;
        description = "Workspace source root to plan/build.";
      };

      manifestPath = lib.mkOption {
        type = lib.types.path;
        default = config.src + "/Cargo.toml";
        defaultText = lib.literalExpression "src + \"/Cargo.toml\"";
        description = "Path to the workspace Cargo manifest.";
      };

      cargoHome = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Optional pre-populated CARGO_HOME to use for replay.";
      };

      gitSourceHashes = lib.mkOption {
        type = lib.types.attrsOf lib.types.str;
        default = { };
        description = "Mapping of Cargo git source IDs to fixed-output hashes.";
      };

      allowImpureGitFetch = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Allow impure builtins.fetchGit fallback for unresolved git hashes.";
      };

      release = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Plan and replay in release mode.";
      };

      targetTriple = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Optional Cargo target triple forwarded to planning.";
      };

      target = lib.mkOption {
        type = lib.types.str;
        default = "default";
        description = "Selected build target: default output, package key, or unique crate name.";
      };

      name = lib.mkOption {
        type = lib.types.str;
        default = "nix-cargo-driver";
        description = "Driver derivation name prefix.";
      };
    };
  };

  normalizeArgs = args:
    (lib.evalModules {
      modules = [
        typedArgsModule
        { config = args; }
      ];
    }).config;
in
args:
let
  cfg = normalizeArgs args;
in
pkgs.callPackage driverFile {
  inherit nixCargo;
  inherit (cfg)
    src
    manifestPath
    cargoHome
    gitSourceHashes
    allowImpureGitFetch
    release
    targetTriple
    target
    name;
}

