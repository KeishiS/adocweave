#!/usr/bin/env bash
set -euo pipefail

version="$(node --input-type=module -e "import manifest from './packages/wasm/package.json' with { type: 'json' }; process.stdout.write(manifest.version)")"
# npm packが作るtarballのrootは、versionを含まない``package``に正規化される。
package="package"
source_archive="target/distrib/adocweave-wasm-$version.tgz"
root="$(mktemp -d "${TMPDIR:-/tmp}/adocweave-wasm-bundler.XXXXXX")"
archive="$root/adocweave-wasm-$version-bundled.tgz"
trap 'rm -rf "$root"' EXIT
tar -xzf "$source_archive" -C "$root"
mkdir -p "$root/consumer/node_modules/@adocweave"
ln -s "$root/$package" "$root/consumer/node_modules/@adocweave/wasm"
cp packages/wasm/bundler-entry.mjs "$root/consumer/app.mjs"
cp packages/wasm/typecheck/package-usage.mts "$root/consumer/package-usage.mts"
tsc --noEmit --strict --target ES2022 --module NodeNext --moduleResolution NodeNext \
  --lib DOM,ES2022 "$root/consumer/package-usage.mts"
esbuild "$root/consumer/app.mjs" \
  --bundle --format=esm --platform=browser \
  --outfile="$root/$package/example/app.mjs"
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
  -czf "$archive" -C "$root" "$package"
node tools/wasm-release-smoke.mjs "$archive" "${ADOCWEAVE_BROWSER:-chromium}"
