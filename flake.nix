# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
{
  description = "Gantry: a themed, animated greetd greeter for Steelbore OS Bravais";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: {
        gantry = pkgs.callPackage ./packaging/default.nix { };
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.gantry;
      });

      overlays.default = final: _prev: {
        gantry = final.callPackage ./packaging/default.nix { };
      };

      nixosModules.default = {
        imports = [ ./nix/module.nix ];
        nixpkgs.overlays = [ self.overlays.default ];
      };

      checks = forAllSystems (pkgs: {
        inherit (self.packages.${pkgs.stdenv.hostPlatform.system}) gantry;
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.gantry ];
          packages = [
            pkgs.clippy
            pkgs.rustfmt
            pkgs.cargo-audit
            pkgs.cargo-deny
            pkgs.reuse
          ];
        };
      });
    };
}
