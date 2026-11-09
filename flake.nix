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
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
    in
    {
      devShells = nixpkgs.lib.genAttrs systems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
        in
        {
          # `makers verify`が実行するコマンドをそろえます。
          # 検査を加えて新しいツールが必要になったら、同じ変更でここへ加えます。
          default = pkgs.mkShell {
            packages = [
              # rustupと同じrust-toolchain.tomlを読み、Rustのバージョンをそろえます。
              (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
              pkgs.cargo-make
              pkgs.nodejs_24
              pkgs.git
            ];
          };
        }
      );
    };
}
