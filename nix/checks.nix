# Checks that the published flake keeps working the way installers expect.
{ pkgs, nixpkgs, self, system, packageVersion }:
let
  package = self.packages.${system}.default;
  runtimeClosure = pkgs.closureInfo {
    rootPaths = [ package ];
  };
  nixos = nixpkgs.lib.nixosSystem {
    inherit system;
    modules = [
      {
        environment.systemPackages = [ package ];
        system.stateVersion = "24.11";
      }
    ];
  };
in
{
  package = package;

  # The assertions fix the names this flake publishes. They hold today because
  # every output is the same derivation, and they exist so that a later edit
  # that renames an output or points an app at the wrong binary fails here
  # instead of in someone else's configuration. The script then runs both
  # programs and rejects a runtime closure that dragged a development tool in.
  package-smoke =
    assert self.packages.${system}.adocweave == package;
    assert self.packages.${system}.adocweave-cli == package;
    assert self.packages.${system}.adocweave-lsp == package;
    assert self.apps.${system}.default.program == "${package}/bin/adocweave";
    assert self.apps.${system}.adocweave-lsp.program == "${package}/bin/adocweave-lsp";
    assert builtins.isFunction self.overlays.default;
    pkgs.runCommand "adocweave-package-smoke"
      {
        nativeBuildInputs = [ pkgs.jq ];
      } ''
      test "$(${package}/bin/adocweave --version --json | jq -r .packageVersion)" = "${packageVersion}"
      test "$(${package}/bin/adocweave-lsp --version --json | jq -r .packageVersion)" = "${packageVersion}"
      if grep -E '/[^/]*(chromium|nodejs|rustc|cargo)-' ${runtimeClosure}/store-paths; then
        echo "development or browser tool found in the AdocWeave runtime closure" >&2
        exit 1
      fi
      touch "$out"
    '';

  nixos-package-evaluation =
    assert builtins.elem package nixos.config.environment.systemPackages;
    pkgs.runCommand "adocweave-nixos-package-evaluation" { } ''
      test -x ${package}/bin/adocweave
      test -x ${package}/bin/adocweave-lsp
      touch "$out"
    '';
}
