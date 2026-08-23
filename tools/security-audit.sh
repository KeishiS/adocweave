#!/usr/bin/env bash
set -euo pipefail

readonly root="$(git rev-parse --show-toplevel)"
cd "$root"

cargo deny --manifest-path Cargo.toml --all-features check --config deny.toml --exclude-dev advisories licenses bans sources
cargo deny --manifest-path editors/zed/Cargo.toml --all-features check --config deny.toml --exclude-dev advisories licenses bans sources
npm audit --omit=dev --prefix editors/vscode
node --test tools/verify-advisory-exceptions.test.mjs
yq -p toml -o json deny.toml | node tools/verify-advisory-exceptions.mjs
node tools/verify-vscode-dependencies.mjs
