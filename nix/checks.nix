# Build and smoke-test the single package published by the flake.
{
  pkgs,
  package,
  cliVersion,
  lspVersion,
}:
let
  runtimeClosure = pkgs.closureInfo {
    rootPaths = [ package ];
  };
in
{
  default =
    pkgs.runCommand "adocweave-package-check"
      {
        nativeBuildInputs = [ pkgs.jq ];
      }
      ''
        test "$(${package}/bin/adocweave --version --json | jq -r .packageVersion)" = "${cliVersion}"
        test "$(${package}/bin/adocweave-lsp --version --json | jq -r .packageVersion)" = "${lspVersion}"
        if grep -E '/[^/]*(chromium|nodejs|rust-minimal|rustc|cargo)-' ${runtimeClosure}/store-paths; then
          echo "development or browser tool found in the AdocWeave runtime closure" >&2
          exit 1
        fi
        mkdir "$out"
        ln -s ${package} "$out/package"
      '';
}
