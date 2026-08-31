{
  description = "AdocWeave CLI, Language Server, and development environment";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      ...
    }:
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

      toolchains = builtins.fromJSON (builtins.readFile ./toolchains.json);
      inherit (toolchains) rustVersion nodeVersion;
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

      toolchain = import ./nix/toolchain.nix { inherit nixpkgs rust-overlay; };
      inherit (toolchain)
        mkPkgs
        stableRust
        developmentRust
        ciRust
        ;
    in
    {
      packages = forAllPackageSystems (
        system:
        let
          pkgs = mkPkgs system;
        in
        {
          default = import ./nix/package.nix {
            inherit pkgs;
            src = self;
            inherit version rustVersion stableRust;
          };
        }
      );

      apps = forAllPackageSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/adocweave";
          meta.description = "Run the AdocWeave command-line converter";
        };
      });

      checks = forAllPackageSystems (
        system:
        import ./nix/checks.nix {
          pkgs = mkPkgs system;
          package = self.packages.${system}.default;
          inherit version;
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
