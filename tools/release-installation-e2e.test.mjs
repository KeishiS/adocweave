import assert from "node:assert/strict";
import test from "node:test";

import {
  missingInstallationAssets,
  requiredProductInstallationAssets,
} from "./platform-contract.mjs";

const target = "x86_64-unknown-linux-musl";
const version = "0.46.2";

for (const [product, expected] of [
  ["cli", `adocweave-cli-${target}.zip`],
  ["lsp", `adocweave-lsp-${target}.zip`],
  ["browser", `adocweave-browser-${version}.tar.xz`],
  ["textlint", `adocweave-textlint-plugin-asciidoc-${version}.tgz`],
  ["vscode", `adocweave-vscode-${version}.vsix`],
  ["zed", `adocweave-zed-${version}.tar.xz`],
]) {
  test(`${product}は自製品の1 assetだけを要求する`, () => {
    const required = requiredProductInstallationAssets(product, target, version, "zip");
    assert.deepEqual(required, [expected]);
    assert.deepEqual(missingInstallationAssets([], required), [expected]);
    assert.deepEqual(missingInstallationAssets([expected], required), []);
  });
}

test("未知の製品を拒否する", () => {
  assert.throws(
    () => requiredProductInstallationAssets("unknown", target, version, "zip"),
    /unsupported installation product/,
  );
});
