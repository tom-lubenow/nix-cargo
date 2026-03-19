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

      perSystem = { self', pkgs, system, ... }: let
        craneLib = inputs.crane.mkLib pkgs;
        src = ./.;
        baseCommon = {
          src = src;
          doCheck = false;
          cargoExtraArgs = "--locked";
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.openssl ];
        };
        cargoVendorDir = craneLib.vendorCargoDeps baseCommon;
        common = baseCommon // {
          inherit cargoVendorDir;
        };
        cargoArtifacts = craneLib.buildDepsOnly (common);
        driverCargoHome = pkgs.runCommand "nix-cargo-driver-cargo-home" {} ''
          mkdir -p "$out"
          cp ${cargoVendorDir}/config.toml "$out/config.toml"
          chmod u+w "$out/config.toml"
          cat >> "$out/config.toml" <<EOF
[net]
offline = true
EOF
        '';
        mkDriver = import ./lib/mk-driver.nix {
          lib = pkgs.lib;
          inherit pkgs;
          nixCargo = self'.packages.default;
        };
        benchmarkBaselineCheck = pkgs.writeShellApplication {
          name = "benchmark-baseline-check";
          runtimeInputs = [ pkgs.jq pkgs.nix self'.packages.default ];
          text = ''
            export NIX_CARGO_BIN="${self'.packages.default}/bin/nix-cargo"
            exec ${src}/examples/incremental-benchmark-baseline-check.sh \
              --engine nix-cargo \
              --no-warmup \
              "$@"
          '';
        };
        benchmarkMatrixBaselineCheck = pkgs.writeShellApplication {
          name = "benchmark-matrix-baseline-check";
          runtimeInputs = [ pkgs.jq pkgs.nix self'.packages.default ];
          text = ''
            export NIX_CARGO_BIN="${self'.packages.default}/bin/nix-cargo"
            exec ${src}/examples/incremental-benchmark-matrix-baseline-check.sh \
              --engine nix-cargo \
              --no-warmup \
              "$@"
          '';
        };
        benchmarkCiChecks = pkgs.writeShellApplication {
          name = "benchmark-ci-checks";
          runtimeInputs = [ pkgs.jq pkgs.nix self'.packages.default ];
          text = ''
            export NIX_CARGO_BIN="${self'.packages.default}/bin/nix-cargo"
            ${src}/examples/incremental-benchmark-baseline-check.sh --engine nix-cargo --no-warmup
            ${src}/examples/incremental-benchmark-matrix-baseline-check.sh --engine nix-cargo --no-warmup
          '';
        };
        benchmarkCiChecksCargo2nix = pkgs.writeShellApplication {
          name = "benchmark-ci-checks-cargo2nix";
          runtimeInputs = [ pkgs.jq pkgs.nix self'.packages.default ];
          text = ''
            if [ "''${NIX_CARGO_ENABLE_CARGO2NIX_LANE:-0}" != "1" ]; then
              echo "benchmark-ci-checks-cargo2nix: skipped (set NIX_CARGO_ENABLE_CARGO2NIX_LANE=1 to enable)"
              exit 0
            fi
            export NIX_CARGO_BIN="${self'.packages.default}/bin/nix-cargo"
            ${src}/examples/incremental-benchmark-baseline-check.sh --engine both --no-warmup
            ${src}/examples/incremental-benchmark-matrix-baseline-check.sh --engine both --no-warmup
          '';
        };
      in {
        packages = {
          default = craneLib.buildPackage (common // { inherit cargoArtifacts; });
          driver-default = mkDriver {
            inherit src;
            cargoHome = driverCargoHome;
            name = "nix-cargo-driver-default";
            target = "default";
          };
          benchmark-baseline-check = benchmarkBaselineCheck;
          benchmark-matrix-baseline-check = benchmarkMatrixBaselineCheck;
          benchmark-ci-checks = benchmarkCiChecks;
          benchmark-ci-checks-cargo2nix = benchmarkCiChecksCargo2nix;
        };

        apps = {
          benchmark-baseline-check = {
            type = "app";
            program = "${self'.packages.benchmark-baseline-check}/bin/benchmark-baseline-check";
          };
          benchmark-matrix-baseline-check = {
            type = "app";
            program = "${self'.packages.benchmark-matrix-baseline-check}/bin/benchmark-matrix-baseline-check";
          };
          benchmark-ci-checks = {
            type = "app";
            program = "${self'.packages.benchmark-ci-checks}/bin/benchmark-ci-checks";
          };
          benchmark-ci-checks-cargo2nix = {
            type = "app";
            program = "${self'.packages.benchmark-ci-checks-cargo2nix}/bin/benchmark-ci-checks-cargo2nix";
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
          inputsFrom = [ self'.packages.default ];
          packages = [ pkgs.rustc pkgs.cargo pkgs.cargo-edit pkgs.pkg-config pkgs.openssl ];
        };
      };
    };
}
