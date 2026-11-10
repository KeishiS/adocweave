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
      # `makers verify`が実行するコマンドをそろえます。
      # 検査を加えて新しいツールが必要になったら、同じ変更でここへ加えます。
      devShells.${system}.default = pkgs.mkShell {
        packages = [
          # rust-toolchain.tomlが定めるバージョンのRustを、コンポーネントと構築先を含めて導入します。
          (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
          pkgs.cargo-make
          pkgs.nodejs_24
          pkgs.git
        ];
      };
    };
}
