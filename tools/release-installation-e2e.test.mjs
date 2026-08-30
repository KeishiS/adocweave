import assert from "node:assert/strict";
import test from "node:test";

import {
  missingInstallationAssets,
  requiredInstallationAssets,
} from "./platform-contract.mjs";

const target = "x86_64-unknown-linux-musl";
const version = "0.46.2";

for (const [kind, expected] of [
  ["native", `adocweave-${target}.zip`],
  ["wasm", `adocweave-wasm-${version}.tgz`],
  ["vscode", `adocweave-vscode-${version}.vsix`],
]) {
  test(`${kind}は対応する一つのassetだけを要求する`, () => {
    const required = requiredInstallationAssets(kind, target, version);
    assert.deepEqual(required, [expected]);
    assert.deepEqual(missingInstallationAssets([], required), [expected]);
    assert.deepEqual(missingInstallationAssets([expected], required), []);
  });
}

test("未知の導入種別を拒否する", () => {
  assert.throws(
    () => requiredInstallationAssets("unknown", target, version),
    /unsupported installation kind/,
  );
});
