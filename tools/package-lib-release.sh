#!/usr/bin/env bash
# Rustライブラリのsourceを決定的なarchiveへまとめる。crates.ioへは公開しないため、
# GitHub Releaseのarchiveが版付きの配布物になる。
set -euo pipefail

version="$(node --input-type=module -e "import { readFileSync } from 'node:fs'; const source = readFileSync('./Cargo.toml', 'utf8'); process.stdout.write(/^\[workspace\.package\][\s\S]*?^version = \"([^\"]+)\"/m.exec(source)[1])")"
package="adocweave-lib-$version"
stage="target/lib-release/$package"
archive="target/distrib/$package.tar.xz"

# workspace versionを共有するcrateだけを収める。製品別の版を持つcrateは含めない。
crates=(adocweave adocweave-config adocweave-host adocweave-textlint adocweave-workspace)

rm -rf "target/lib-release"
mkdir -p "$stage/crates" "target/distrib"
cp Cargo.toml Cargo.lock "$stage/"
cp LICENSE-MIT LICENSE-APACHE "$stage/"
for crate in "${crates[@]}"; do
  mkdir -p "$stage/crates/$crate"
  cp -R "crates/$crate/." "$stage/crates/$crate/"
done
node tools/generate-third-party-notices.mjs "$stage/THIRD_PARTY_NOTICES.adoc"

# 収録するsourceは追跡済みのfileだけで、機械固有pathの混入経路は生成物に限られる。
# source中の絶対pathは試験のfixture文字列なので、検査対象へ含めない。
if LC_ALL=C grep -a -E '(/workspace/|/home/|/tmp/)' "$stage/THIRD_PARTY_NOTICES.adoc" >/dev/null; then
  echo "library release notices contain a machine-local absolute path" >&2
  exit 1
fi

tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
  -cJf "$archive" -C target/lib-release "$package"
test -s "$archive"
echo "library release artifact: $archive"
