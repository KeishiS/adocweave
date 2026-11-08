import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";

const manifest = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"));
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
  assert.deepEqual(manifest.peerDependencies, {
    "@textlint/types": "^15.8.0",
    textlint: "^15.8.0",
  });
});

test("npmが描画するMarkdownのREADMEを収録する", () => {
  assert.ok(manifest.files.includes("README.md"));
  assert.equal(existsSync(new URL("./README.md", import.meta.url)), true);
});

test("licenseと第三者依存通知を収録する", () => {
  for (const license of ["LICENSE-APACHE", "LICENSE-MIT"]) {
    assert.equal(existsSync(new URL(`./${license}`, import.meta.url)), true);
  }
  assert.ok(manifest.files.includes("THIRD_PARTY_NOTICES.adoc"));
});
