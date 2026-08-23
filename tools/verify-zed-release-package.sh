#!/usr/bin/env bash
set -euo pipefail

version="$(node --input-type=module -e "import { readFileSync } from 'node:fs'; const source = readFileSync('./editors/zed/extension.toml', 'utf8'); process.stdout.write(/^version = \"([^\"]+)\"/m.exec(source)[1])")"
archive="target/distrib/adocweave-zed-$version.tar.xz"
first="$(sha256sum "$archive" | cut -d ' ' -f 1)"
bash tools/package-zed-release.sh >/dev/null
second="$(sha256sum "$archive" | cut -d ' ' -f 1)"
if [[ "$first" != "$second" ]]; then
  echo "Zed release archive is not deterministic" >&2
  exit 1
fi
node tools/zed-release-smoke.mjs "$archive"
