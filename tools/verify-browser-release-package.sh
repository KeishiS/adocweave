#!/usr/bin/env bash
set -euo pipefail

version="$(node --input-type=module -e "import manifest from './web-worker/package.json' with { type: 'json' }; process.stdout.write(manifest.version)")"
# npm packが作るtarballのrootは、versionを含まない``package``に正規化される。
package="package"
archive="target/distrib/adocweave-browser-$version.tgz"
first="$(sha256sum "$archive" | cut -d ' ' -f 1)"
bash tools/package-browser-release.sh >/dev/null
second="$(sha256sum "$archive" | cut -d ' ' -f 1)"
if [[ "$first" != "$second" ]]; then
  echo "browser release archive is not deterministic" >&2
  exit 1
fi

actual="$(mktemp "${TMPDIR:-/tmp}/adocweave-browser-archive.XXXXXX.list")"
expected="$(mktemp "${TMPDIR:-/tmp}/adocweave-browser-archive-expected.XXXXXX.list")"
root="$(mktemp -d "${TMPDIR:-/tmp}/adocweave-browser-import.XXXXXX")"
trap 'rm -f "$actual" "$expected"; rm -rf "$root"' EXIT
tar -tzf "$archive" | LC_ALL=C sort > "$actual"
printf '%s\n' \
  "$package/LICENSE-APACHE" \
  "$package/LICENSE-MIT" \
  "$package/README.md" \
  "$package/THIRD_PARTY_NOTICES.adoc" \
  "$package/package.json" \
  "$package/example/app.mjs" \
  "$package/example/index.html" \
  "$package/wasm/adocweave_wasm.d.ts" \
  "$package/wasm/adocweave_wasm.js" \
  "$package/wasm/adocweave_wasm_bg.wasm" \
  "$package/worker/client.mjs" \
  "$package/worker/contracts.mjs" \
  "$package/worker/index.d.mts" \
  "$package/worker/index.mjs" \
  "$package/worker/protocol.d.mts" \
  "$package/worker/worker-protocol.mjs" \
  "$package/worker/worker.mjs" | LC_ALL=C sort > "$expected"
diff -u "$expected" "$actual"
tar -xzf "$archive" -C "$root"
node --input-type=module -e '
  const publicApi = await import(process.argv[1]);
  const protocol = await import(process.argv[2]);
  if (publicApi.PROTOCOL_SCHEMA_VERSION !== protocol.PROTOCOL_SCHEMA_VERSION) {
    throw new Error("browser archive protocol schema version mismatch");
  }
' "$root/$package/worker/index.mjs" "$root/$package/worker/worker-protocol.mjs"
echo "browser release package verified: $second"
