{
  description = "AdocWeave CLI, Language Server, and development environment";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    { self, nixpkgs, rust-overlay, ... }:
    let
      # The package is published for Linux only. Shells are also built on macOS
      # because development and part of CI happen there.
      packageSystems = [
        "aarch64-linux"
        "x86_64-linux"
      ];
      supportedSystems = packageSystems ++ [ "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      forAllPackageSystems = nixpkgs.lib.genAttrs packageSystems;

      # The release manifest is the single record of the versions the release
      # train is built with. Everything below reads them from there.
      releaseManifest = builtins.fromJSON (builtins.readFile ./release-manifest.json);
      inherit (releaseManifest) packageVersion rustVersion nodeVersion;

      toolchain = import ./nix/toolchain.nix { inherit nixpkgs rust-overlay; };
      inherit (toolchain) mkPkgs stableRust developmentRust ciRust;
    in
    {
      overlays.default = final: _previous: {
        adocweave = import ./nix/package.nix {
          pkgs = final;
          src = self;
          inherit packageVersion rustVersion stableRust;
        };
      };

      packages = forAllPackageSystems (
        system:
        let
          package = ((mkPkgs system).extend self.overlays.default).adocweave;
        in
        {
          default = package;
          adocweave = package;
          adocweave-cli = package;
          adocweave-lsp = package;
        }
      );

      apps = forAllPackageSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/adocweave";
          meta.description = "Run the AdocWeave command-line converter";
        };
        adocweave-lsp = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/adocweave-lsp";
          meta.description = "Run the AdocWeave Language Server";
        };
      });

      checks = forAllPackageSystems (
        system:
        import ./nix/checks.nix {
          pkgs = mkPkgs system;
          inherit nixpkgs self system packageVersion;
        }
      );

      devShells = forAllSystems (
        system:
        import ./nix/devshells.nix {
          pkgs = mkPkgs system;
          inherit nodeVersion developmentRust ciRust;
        }
      );
    };
}
