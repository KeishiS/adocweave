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
    noticeDir="$out/share/doc/adocweave/browser-assets"
    mkdir -p "$noticeDir"
    install -m644 crates/adocweave/assets/revealjs/LICENSE.* crates/adocweave/assets/revealjs/NOTICE.txt "$noticeDir/"
    cslNoticeDir="$out/share/doc/adocweave/csl"
    mkdir -p "$cslNoticeDir"
    install -m644 crates/adocweave/assets/slides/LICENSE.csl-locales.txt \
      crates/adocweave/assets/slides/NOTICE.csl-locales.txt \
      crates/adocweave/assets/slides/locale-en-US.xml "$cslNoticeDir/"
    runHook postInstall
  '';
  meta = {
    description = "AsciiDoc converter and Language Server";
    homepage = "https://github.com/KeishiS/adocweave";
    license = with pkgs.lib.licenses; [ asl20 mit cc-by-sa-30 ];
    mainProgram = "adocweave";
    platforms = pkgs.lib.platforms.linux;
  };
}
