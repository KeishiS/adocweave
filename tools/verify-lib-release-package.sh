#!/usr/bin/env bash
# archiveが決定的で、workspace versionを共有するcrateだけを収めていることを確かめる。
set -euo pipefail

version="$(node --input-type=module -e "import { readFileSync } from 'node:fs'; const source = readFileSync('./Cargo.toml', 'utf8'); process.stdout.write(/^\[workspace\.package\][\s\S]*?^version = \"([^\"]+)\"/m.exec(source)[1])")"
package="adocweave-lib-$version"
archive="target/distrib/$package.tar.xz"
first="$(sha256sum "$archive" | cut -d ' ' -f 1)"
bash tools/package-lib-release.sh >/dev/null
second="$(sha256sum "$archive" | cut -d ' ' -f 1)"
if [[ "$first" != "$second" ]]; then
  echo "library release archive is not deterministic" >&2
  exit 1
fi

# 一覧は一度だけ取る。grep -qへ直接つなぐとtarがSIGPIPEで終わり、pipefailが失敗と判定する。
listing="$(tar -tJf "$archive")"
members="$(printf '%s\n' "$listing" | sed -n "s|^$package/crates/\([^/]*\)/.*|\1|p" | LC_ALL=C sort -u)"
expected="$(printf '%s\n' adocweave adocweave-config adocweave-host adocweave-textlint adocweave-workspace)"
if [[ "$members" != "$expected" ]]; then
  echo "library release archive contains unexpected crates:" >&2
  diff -u <(printf '%s\n' "$expected") <(printf '%s\n' "$members") >&2 || true
  exit 1
fi

for required in "$package/Cargo.toml" "$package/Cargo.lock" "$package/LICENSE-MIT" \
  "$package/LICENSE-APACHE" "$package/THIRD_PARTY_NOTICES.adoc" \
  "$package/crates/adocweave/src/lib.rs"; do
  if ! printf '%s\n' "$listing" | grep -qxF "$required"; then
    echo "library release archive is missing $required" >&2
    exit 1
  fi
done

# 製品別の版を持つcrateを取り込むと、版の正本が二重になる。
for excluded in adocweave-cli adocweave-lsp adocweave-wasm adocweave-textlint-wasm; do
  if printf '%s\n' "$listing" | grep -q "^$package/crates/$excluded/"; then
    echo "library release archive must not contain $excluded" >&2
    exit 1
  fi
done

echo "library release package verified: $second"
