#!/usr/bin/env bash
set -euo pipefail

version="$(node --input-type=module -e "import manifest from './web-worker/package.json' with { type: 'json' }; process.stdout.write(manifest.version)")"
archive_name="adocweave-wasm-$version.tgz"
archive="target/distrib/$archive_name"
npm_cache="${ADOCWEAVE_WASM_NPM_CACHE:-target/npm-cache}"
# npm packはtarballのrootを``package/``へ正規化する。stageもその名前で作る。
scratch="$(mktemp -d "${TMPDIR:-/tmp}/adocweave-wasm.XXXXXX")"
stage="$scratch/package"
trap 'rm -rf "$scratch"' EXIT

export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$(pwd)=. --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=cargo-home"

cargo build -p adocweave-wasm --profile wasm --target wasm32-unknown-unknown

if command -v wasm-bindgen >/dev/null 2>&1; then
  wasm_bindgen="$(command -v wasm-bindgen)"
else
  tool_root="target/release-tools/wasm-bindgen-cli-0.2.121"
  cargo install --locked wasm-bindgen-cli --version 0.2.121 --root "$tool_root"
  wasm_bindgen="$tool_root/bin/wasm-bindgen"
fi
"$wasm_bindgen" \
  --target web \
  --out-dir target/adocweave-wasm \
  target/wasm32-unknown-unknown/wasm/adocweave_wasm.wasm

mkdir -p "$stage/wasm" "$stage/worker" "$stage/example"
cp target/adocweave-wasm/adocweave_wasm.js "$stage/wasm/"
cp target/adocweave-wasm/adocweave_wasm_bg.wasm "$stage/wasm/"
if [[ -f target/adocweave-wasm/adocweave_wasm.d.ts ]]; then
  cp target/adocweave-wasm/adocweave_wasm.d.ts "$stage/wasm/"
fi
cp web-worker/analysis.mjs web-worker/client.mjs web-worker/contracts.mjs \
  web-worker/direct.mjs web-worker/direct.d.mts web-worker/index.mjs \
  web-worker/index.d.mts web-worker/protocol.d.mts web-worker/worker-protocol.mjs \
  web-worker/worker.mjs "$stage/worker/"
cp web-worker/example/index.html web-worker/example/app.mjs "$stage/example/"
cp web-worker/package.json web-worker/README.md LICENSE-MIT LICENSE-APACHE "$stage/"
node tools/generate-third-party-notices.mjs "$stage/THIRD_PARTY_NOTICES.adoc"

mkdir -p target/distrib
pack_result="$(npm --cache "$npm_cache" pack --ignore-scripts --json --pack-destination "$scratch" "$stage")"
packed_name="$(node --input-type=module -e '
  const result = JSON.parse(process.argv[1]);
  if (!Array.isArray(result) || result.length !== 1) throw new Error("npm pack produced an unexpected result");
  process.stdout.write(result[0].filename);
' "$pack_result")"
if [[ "$packed_name" != "$archive_name" ]]; then
  echo "unexpected WebAssembly archive name: $packed_name" >&2
  exit 1
fi
cp "$scratch/$packed_name" "$archive"
if tar -xOzf "$archive" | LC_ALL=C grep -a -E '(/workspace/|/home/|/tmp/)' >/dev/null; then
  echo "WebAssembly release artifact contains a machine-local absolute path" >&2
  exit 1
fi
echo "WebAssembly release artifact: $archive"
