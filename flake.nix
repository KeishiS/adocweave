{
  description = "Development environment for AdocWeave";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = [
          (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
          pkgs.cargo-make
          pkgs.nodejs_24
          pkgs.git

          # formatter & linter
          pkgs.biome
          pkgs.rumdl
          pkgs.tombi
          pkgs.yamlfmt
          pkgs.yamllint
          pkgs.actionlint
          pkgs.nixfmt
        ];
      };
    };
}
