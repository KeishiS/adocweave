# The released native package, built from this repository's own Cargo.lock.
{ pkgs, src, version, rustVersion, stableRust }:
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
  inherit version;
  inherit src;
  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [
    "-p=adocweave"
  ];
  doCheck = false;
  strictDeps = true;
  installPhase = ''
    runHook preInstall
    releaseDir="target/${pkgs.stdenv.hostPlatform.rust.rustcTarget}/release"
    install -Dm755 "$releaseDir/adocweave" "$out/bin/adocweave"
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
