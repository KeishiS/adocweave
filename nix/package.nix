# The released native package: the command-line converter and the Language
# Server, built from this repository's own Cargo.lock.
{ pkgs, src, cliVersion, rustVersion, stableRust }:
let
  rust = stableRust pkgs;
  rustPlatform = pkgs.makeRustPlatform {
    cargo = rust;
    rustc = rust;
  };
in
# The toolchain manifest names the Rust version used for reproducible builds.
# nixpkgs moves on its own schedule, so the build stops rather than shipping a
# package compiled by a version this repository has not declared.
assert rust.version == rustVersion;
rustPlatform.buildRustPackage {
  pname = "adocweave";
  version = cliVersion;
  inherit src;
  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [
    "-p=adocweave-cli"
    "-p=adocweave-lsp"
  ];
  doCheck = false;
  strictDeps = true;
  installPhase = ''
    runHook preInstall
    releaseDir="target/${pkgs.stdenv.hostPlatform.rust.rustcTarget}/release"
    install -Dm755 "$releaseDir/adocweave" "$out/bin/adocweave"
    install -Dm755 "$releaseDir/adocweave-lsp" "$out/bin/adocweave-lsp"
    runHook postInstall
  '';
  meta = {
    description = "AsciiDoc converter and Language Server";
    homepage = "https://github.com/KeishiS/adocweave";
    license = with pkgs.lib.licenses; [ asl20 mit ];
    mainProgram = "adocweave";
    platforms = pkgs.lib.platforms.linux;
  };
}
