# Build and smoke-test the single package published by the flake.
{
  pkgs,
  package,
  version,
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
        test "$(${package}/bin/adocweave --version --json | jq -r .packageVersion)" = "${version}"
        noticeDir="${package}/share/doc/adocweave/browser-assets"
        test -s "$noticeDir/NOTICE.txt"
        for name in LICENSE.fitty.txt LICENSE.marked.txt LICENSE.revealjs.txt; do
          test -s "$noticeDir/$name"
          grep -F 'Permission is hereby granted, free of charge' "$noticeDir/$name" > /dev/null
        done
        cslNoticeDir="${package}/share/doc/adocweave/csl"
        test -s "$cslNoticeDir/locale-en-US.xml"
        grep -F 'https://github.com/citation-style-language/locales' "$cslNoticeDir/NOTICE.csl-locales.txt" > /dev/null
        grep -F 'https://creativecommons.org/licenses/by-sa/3.0/' "$cslNoticeDir/NOTICE.csl-locales.txt" > /dev/null
        test -s "$cslNoticeDir/LICENSE.csl-locales.txt"
        if grep -E '/[^/]*(chromium|nodejs|rust-minimal|rustc|cargo)-' ${runtimeClosure}/store-paths; then
          echo "development or browser tool found in the AdocWeave runtime closure" >&2
          exit 1
        fi
        mkdir "$out"
        ln -s ${package} "$out/package"
      '';
}
