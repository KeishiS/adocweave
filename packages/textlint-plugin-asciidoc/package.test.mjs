import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";

import {
  satisfiesPeerRange,
  verifiedTextlintVersion
} from "../../tools/textlint-plugin-package.mjs";

const manifest = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"));
const toolchains = JSON.parse(readFileSync(new URL("../../toolchains.json", import.meta.url), "utf8"));
const bridge = readFileSync(new URL("./bridge.mjs", import.meta.url), "utf8");
const changelog = readFileSync(new URL("./CHANGELOG.md", import.meta.url), "utf8");

test("public registryへ公開しruntime npm依存を持たない", () => {
  assert.equal(manifest.name, "@adocweave/textlint-plugin-asciidoc");
  assert.equal(manifest.private, undefined);
  assert.equal(manifest.publishConfig.access, "public");
  assert.deepEqual(manifest.dependencies, undefined);
});

test("package.jsonのversionを専用Changelogへ記録する", () => {
  assert.match(manifest.version, /^\d+\.\d+\.\d+$/u);
  assert.match(changelog, new RegExp(`^## \\[${manifest.version.replaceAll(".", "\\.")}\\]`, "mu"));
  assert.ok(manifest.files.includes("CHANGELOG.md"));
});

test("検証した組合せを下限とする範囲で依存を受け入れる", () => {
  // 完全一致で固定すると、npm利用者の環境を検証対象より狭く縛る。
  assert.equal(manifest.engines.node, `>=${toolchains.nodeVersion}`);
  assert.deepEqual(manifest.peerDependencies, {
    "@textlint/types": "^15.8.0",
    textlint: "^15.8.0",
  });
});

test("継続的に検査する組合せが受け入れる範囲に含まれる", () => {
  const pinned = verifiedTextlintVersion();
  assert.match(pinned, /^\d+\.\d+\.\d+$/u);
  assert.ok(satisfiesPeerRange(pinned, manifest.peerDependencies.textlint));
  assert.ok(satisfiesPeerRange(pinned, manifest.peerDependencies["@textlint/types"]));
});

test("受け入れる範囲の外側を拒否する", () => {
  assert.equal(satisfiesPeerRange("15.8.0", "^15.8.0"), true);
  assert.equal(satisfiesPeerRange("15.9.3", "^15.8.0"), true);
  assert.equal(satisfiesPeerRange("15.7.9", "^15.8.0"), false);
  assert.equal(satisfiesPeerRange("16.0.0", "^15.8.0"), false);
  assert.equal(satisfiesPeerRange("15.8.0", "15.8.0"), true);
  assert.equal(satisfiesPeerRange("15.8.1", "15.8.0"), false);
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
