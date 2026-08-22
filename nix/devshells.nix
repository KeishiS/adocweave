# Development and CI shells.
#
# CI jobs run in the smallest shell that can finish their gate. A job downloads
# and realizes every package its shell names, so a tool that only one gate needs
# is kept out of the shared set rather than paid for by all of them.
{ pkgs, nodeVersion, developmentRust, ciRust }:
let
  inherit (pkgs) lib stdenv;

  # Runners without Nix read this version from the release manifest and hand it
  # to setup-node. A moving devShell nodejs would split the two, so the shell
  # refuses to build unless they agree.
  checkedNodejs = assert pkgs.nodejs.version == nodeVersion; pkgs.nodejs;

  # cargo-fuzz needs a nightly toolchain of roughly 1.5 GiB. Wrapping it in a
  # script keeps that toolchain out of the shell's own PATH, so the rest of the
  # session still uses the pinned stable compiler.
  fuzzRust = pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default);
  adocweave-fuzz = pkgs.writeShellScriptBin "adocweave-fuzz" ''
    export PATH=${fuzzRust}/bin:${pkgs.cargo-fuzz}/bin:$PATH
    exec cargo fuzz "$@"
  '';

  commonPackages = with pkgs; [
    actionlint
    cargo-dist
    cargo-audit
    cargo-deny
    cargo-make
    curl
    dejavu_fonts
    esbuild
    fontconfig
    gh
    git
    gnutar
    jq
    lld
    checkedNodejs
    typescript
    ripgrep
    stdenv.cc
    wasm-bindgen-cli
    xz
    yq-go
    zip
    unzip
  ];

  # The VS Code extension host is a downloaded Electron binary rather than a Nix
  # package, so it looks for these libraries at run time by name.
  vscodeRuntime = with pkgs; [
    alsa-lib
    at-spi2-atk
    cairo
    cups
    dbus
    expat
    glib
    gtk3
    libgbm
    libdrm
    libxkbcommon
    mesa
    nspr
    nss
    pango
    systemd
    libx11
    libxcomposite
    libxdamage
    libxext
    libxfixes
    libxrandr
    libxcb
  ];

  shell = { rust, extra ? [ ], rustSource ? false }: pkgs.mkShell ({
    packages = commonPackages ++ [ rust ] ++ extra;
    ADOCWEAVE_DIST_BIN = "${pkgs.cargo-dist}/bin/dist";
  } // lib.optionalAttrs rustSource {
    RUST_SRC_PATH = "${rust}/lib/rustlib/src/rust/library";
  } // lib.optionalAttrs stdenv.isLinux {
    LD_LIBRARY_PATH = lib.makeLibraryPath vscodeRuntime;
  });
in
{
  # Everything `cargo make verify` runs, so a local pull request gate matches CI.
  default = shell {
    rust = developmentRust pkgs;
    rustSource = true;
    extra = [ pkgs.rust-analyzer adocweave-fuzz ]
      ++ lib.optionals stdenv.isLinux [ pkgs.chromium pkgs.xvfb ];
  };

  ci = shell {
    rust = ciRust pkgs;
    extra = lib.optionals stdenv.isLinux [ pkgs.xvfb ];
  };

  ci-fuzz = shell {
    rust = ciRust pkgs;
    extra = [ adocweave-fuzz ];
  };

  # The browser acceptance gate must run against a browser this repository pins,
  # not whichever one the runner image happens to carry.
  ci-browser = shell {
    rust = ciRust pkgs;
    extra = lib.optionals stdenv.isLinux [ pkgs.chromium pkgs.xvfb ];
  };

  # HTML validation needs no compiler at all.
  html5 = pkgs.mkShell {
    packages = [
      checkedNodejs
      pkgs.validator-nu
    ];
    ADOCWEAVE_HTML_VALIDATOR = "${pkgs.validator-nu}/bin/vnu";
  };
}
