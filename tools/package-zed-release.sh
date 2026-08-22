#!/usr/bin/env bash
set -euo pipefail

version="$(node --input-type=module -e "import { readFileSync } from 'node:fs'; const source = readFileSync('./editors/zed/extension.toml', 'utf8'); process.stdout.write(/^version = \"([^\"]+)\"/m.exec(source)[1])")"
package="adocweave-zed-$version"
stage="target/zed-release/$package"
archive="target/distrib/$package.tar.xz"

rm -rf "target/zed-release"
mkdir -p "$stage/src" "$stage/languages" "target/distrib"
cp editors/zed/Cargo.toml editors/zed/Cargo.lock editors/zed/extension.toml "$stage/"
cp editors/zed/src/*.rs "$stage/src/"
cp -R editors/zed/languages/. "$stage/languages/"
cp editors/zed/README.adoc "$stage/"
cp LICENSE-MIT LICENSE-APACHE "$stage/"
node tools/generate-third-party-notices.mjs "$stage/THIRD_PARTY_NOTICES.adoc"

tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
  -cJf "$archive" -C target/zed-release "$package"
test -s "$archive"
echo "Zed release artifact: $archive"
