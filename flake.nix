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
      in {
        packages = {
          default = craneLib.buildPackage (common // { inherit cargoArtifacts; });
          driver-default = pkgs.callPackage ./driver.nix {
            inherit src;
            nixCargo = self.packages.${system}.default;
            name = "nix-cargo-driver-default";
            target = "default";
          };
        };

        legacyPackages = {
          mkDriver = args:
            pkgs.callPackage ./driver.nix ({
              nixCargo = self.packages.${system}.default;
            } // args);
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.default ];
          packages = [ pkgs.rustc pkgs.cargo pkgs.cargo-edit pkgs.pkg-config ];
        };
      };
    };
}
