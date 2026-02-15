{
  description = "nix-cargo prototype";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    crane.url = "github:ipetkov/crane";
  };

  outputs = inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [ "x86_64-linux" "aarch64-linux" ];

      perSystem = { self, pkgs, system, ... }: let
        craneLib = inputs.crane.lib.${system};
        src = ./.;
        common = {
          src = src;
          doCheck = false;
          cargoExtraArgs = "--locked";
        };
        cargoArtifacts = craneLib.buildDepsOnly (common);
        mkDriver = import ./lib/mk-driver.nix {
          lib = pkgs.lib;
          inherit pkgs;
          nixCargo = self.packages.${system}.default;
        };
        benchmarkBaselineCheck = pkgs.writeShellApplication {
          name = "benchmark-baseline-check";
          runtimeInputs = [ pkgs.jq pkgs.nix self.packages.${system}.default ];
          text = ''
            export NIX_CARGO_BIN="${self.packages.${system}.default}/bin/nix-cargo"
            exec ${src}/examples/incremental-benchmark-baseline-check.sh \
              --engine nix-cargo \
              --no-warmup \
              "$@"
          '';
        };
        benchmarkMatrixBaselineCheck = pkgs.writeShellApplication {
          name = "benchmark-matrix-baseline-check";
          runtimeInputs = [ pkgs.jq pkgs.nix self.packages.${system}.default ];
          text = ''
            export NIX_CARGO_BIN="${self.packages.${system}.default}/bin/nix-cargo"
            exec ${src}/examples/incremental-benchmark-matrix-baseline-check.sh \
              --engine nix-cargo \
              --no-warmup \
              "$@"
          '';
        };
        benchmarkCiChecks = pkgs.writeShellApplication {
          name = "benchmark-ci-checks";
          runtimeInputs = [ pkgs.jq pkgs.nix self.packages.${system}.default ];
          text = ''
            export NIX_CARGO_BIN="${self.packages.${system}.default}/bin/nix-cargo"
            ${src}/examples/incremental-benchmark-baseline-check.sh --engine nix-cargo --no-warmup
            ${src}/examples/incremental-benchmark-matrix-baseline-check.sh --engine nix-cargo --no-warmup
          '';
        };
      in {
        packages = {
          default = craneLib.buildPackage (common // { inherit cargoArtifacts; });
          driver-default = mkDriver {
            inherit src;
            name = "nix-cargo-driver-default";
            target = "default";
          };
          benchmark-baseline-check = benchmarkBaselineCheck;
          benchmark-matrix-baseline-check = benchmarkMatrixBaselineCheck;
          benchmark-ci-checks = benchmarkCiChecks;
        };

        apps = {
          benchmark-baseline-check = {
            type = "app";
            program = "${self.packages.${system}.benchmark-baseline-check}/bin/benchmark-baseline-check";
          };
          benchmark-matrix-baseline-check = {
            type = "app";
            program = "${self.packages.${system}.benchmark-matrix-baseline-check}/bin/benchmark-matrix-baseline-check";
          };
          benchmark-ci-checks = {
            type = "app";
            program = "${self.packages.${system}.benchmark-ci-checks}/bin/benchmark-ci-checks";
          };
        };

        checks = {
          benchmark-baseline-snapshots-schema = pkgs.runCommand "benchmark-baseline-snapshots-schema" {
            nativeBuildInputs = [ pkgs.jq ];
          } ''
            jq -e '
              type == "array" and
              all(.[]; has("engine") and has("phase") and has("derivations"))
            ' ${src}/examples/benchmark-baselines/incremental-small.json > /dev/null
            jq -e '
              type == "array" and
              all(.[]; has("engine") and has("phase") and has("derivations"))
            ' ${src}/examples/benchmark-baselines/incremental-large.json > /dev/null
            jq -e '
              type == "array" and
              all(.[]; has("scenario") and has("workspace") and has("target_crate") and has("mutation_file") and has("engine") and has("phase") and has("derivations"))
            ' ${src}/examples/benchmark-baselines/matrix-repo.json > /dev/null
            touch "$out"
          '';
          benchmark-matrix-scenarios-schema = pkgs.runCommand "benchmark-matrix-scenarios-schema" {
            nativeBuildInputs = [ pkgs.gawk ];
          } ''
            check_file() {
              local file="$1"
              awk -F '\t' '
                NF == 0 { next }
                NF == 2 { next }
                NF == 4 { next }
                { exit 1 }
              ' "$file"
            }
            check_file ${src}/examples/benchmark-matrix-scenarios/large.tsv
            check_file ${src}/examples/benchmark-matrix-scenarios/repo.tsv
            touch "$out"
          '';
        };

        legacyPackages = {
          inherit mkDriver;
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.default ];
          packages = [ pkgs.rustc pkgs.cargo pkgs.cargo-edit pkgs.pkg-config ];
        };
      };
    };
}
