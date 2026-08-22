# Rust toolchains and the package set they come from.
#
# Every output of this flake needs a `pkgs` carrying the rust-overlay, and the
# three toolchains differ only in what an editor or a CI job actually reads.
{ nixpkgs, rust-overlay }:
let
  # Targets are declared once. Cross compilation is decided by the release
  # plan, not by which shell happens to be open, so every toolchain carries
  # the same list.
  targets = [
    "aarch64-unknown-linux-musl"
    "aarch64-apple-darwin"
    "x86_64-pc-windows-msvc"
    "x86_64-unknown-linux-musl"
    "wasm32-unknown-unknown"
    "wasm32-wasip2"
  ];
in
{
  inherit targets;

  mkPkgs = system: import nixpkgs {
    inherit system;
    overlays = [ (import rust-overlay) ];
  };

  # The toolchain the released package is built with. It carries no extension
  # because nothing in a build reads rustfmt or clippy.
  stableRust = pkgs: pkgs.rust-bin.stable.latest.default;

  developmentRust = pkgs: pkgs.rust-bin.stable.latest.default.override {
    extensions = [
      "clippy"
      "rust-src"
      "rustfmt"
    ];
    inherit targets;
  };

  # CI builds and lints only. The prebuilt standard library documentation and
  # the standard library source are editor conveniences, and every job pays
  # for them because rust-overlay outputs are not in the public binary cache.
  ciRust = pkgs: pkgs.rust-bin.stable.latest.minimal.override {
    extensions = [
      "clippy"
      "rustfmt"
    ];
    inherit targets;
  };
}
