import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";

const manifest = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"));
const toolchains = JSON.parse(readFileSync(new URL("../../toolchains.json", import.meta.url), "utf8"));
const bridge = readFileSync(new URL("./bridge.mjs", import.meta.url), "utf8");

test("public registryへ公開しruntime npm依存を持たない", () => {
  assert.equal(manifest.name, "@adocweave/textlint-plugin-asciidoc");
  assert.equal(manifest.private, undefined);
  assert.equal(manifest.publishConfig.access, "public");
  assert.deepEqual(manifest.dependencies, undefined);
});

test("検証した組合せを下限とする範囲で依存を受け入れる", () => {
  // 完全一致で固定すると、npm利用者の環境を検証対象より狭く縛る。
  assert.equal(manifest.engines.node, `>=${toolchains.nodeVersion}`);
  assert.deepEqual(manifest.peerDependencies, {
    "@textlint/types": "^15.8.0",
    textlint: "^15.8.0",
  });
});

test("npmが描画するMarkdownのREADMEを収録する", () => {
  assert.ok(manifest.files.includes("README.md"));
  assert.equal(existsSync(new URL("./README.md", import.meta.url)), true);
  assert.equal(existsSync(new URL("./README.adoc", import.meta.url)), false);
});

test("parseTextとWASMをpackage内で完結させる", () => {
  assert.match(bridge, /require\("\.\/wasm\/adocweave_textlint_wasm\.cjs"\)/);
  assert.match(bridge, /typeof loaded\?\.parseText !== "function"/);
  assert.doesNotMatch(
    bridge,
    /adapterApiVersion|TEXTLINT_ADAPTER_API_VERSION|package\.json|release-manifest|target\//,
  );
});

test("package境界へプロジェクト固有設定を含めない", () => {
  const serialized = JSON.stringify(manifest);
  for (const name of ["japanese-terminology", "preset-ja", "targets.json"]) {
    assert.doesNotMatch(serialized, new RegExp(name));
  }
  for (const license of ["LICENSE-APACHE", "LICENSE-MIT"]) {
    assert.equal(existsSync(new URL(`./${license}`, import.meta.url)), true);
  }
  assert.ok(manifest.files.includes("THIRD_PARTY_NOTICES.adoc"));
  assert.equal(existsSync(new URL("./THIRD_PARTY_NOTICES.adoc", import.meta.url)), false);
});
