import assert from "node:assert/strict";
import test from "node:test";

import {
  expectedPullRequestAssets,
  verifyPullRequestAssets,
} from "./verify-native-pr-candidate.mjs";

const plan = {
  schemaVersion: 3,
  targets: [
    { executableSuffix: "", os: "linux", triple: "x86_64-unknown-linux-musl" },
    { executableSuffix: "", os: "darwin", triple: "aarch64-apple-darwin" },
    { executableSuffix: ".exe", os: "win32", triple: "x86_64-pc-windows-msvc" },
  ],
  products: [
    {
      product: "lsp",
      versionSource: "crates/adocweave-lsp/Cargo.toml#package.version",
      assetKind: "lsp",
      assetName: "adocweave-lsp-{target}.zip",
      archive: "zip",
      executable: "adocweave-lsp{executableSuffix}",
      build: "cargo-dist",
    },
    {
      product: "vscode",
      versionSource: "editors/vscode/package.json#version",
      assetKind: "vscode",
      assetName: "adocweave-vscode-{version}.vsix",
      archive: "vsix",
      executable: null,
      build: "script",
    },
  ],
};

test("native candidateは選択製品のmacOS・Windows assetだけを要求する", () => {
  assert.deepEqual(expectedPullRequestAssets(plan, "lsp"), [
    "adocweave-lsp-aarch64-apple-darwin.zip",
    "adocweave-lsp-x86_64-pc-windows-msvc.zip",
  ]);
});

test("global candidateは選択製品の1 archiveだけを要求する", () => {
  const expected = ["adocweave-vscode-0.46.2.vsix"];
  assert.deepEqual(expectedPullRequestAssets(plan, "vscode"), expected);
  assert.doesNotThrow(() => verifyPullRequestAssets(expected, plan, "vscode"));
  assert.throws(
    () => verifyPullRequestAssets([...expected, "adocweave-zed-0.46.2.tar.xz"], plan, "vscode"),
    /candidate mismatch/,
  );
});

test("candidateの欠落を拒否する", () => {
  assert.throws(() => verifyPullRequestAssets([], plan, "lsp"), /candidate mismatch/);
});
