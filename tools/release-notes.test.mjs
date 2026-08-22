import assert from "node:assert/strict";
import test from "node:test";

import {
  buildReleaseNotes,
  loadReleaseNotesSource,
  parseReleaseNotesSource,
  releaseNoteHeadings,
  releaseNotesTitle,
  renderReleaseNotes,
  validateReleaseNotes,
  validateReleaseNotesSource,
} from "./release-notes.mjs";
import { PRODUCT_IDS, productRelease, relatedApiVersions } from "./release-policy.mjs";
import toolchains from "../toolchains.json" with { type: "json" };

function sampleSource(product, version, overrides = {}) {
  const bodies = {
    "## 主な変更": "- 変更の要点です。",
    "## 対応環境": "対応環境を説明します。",
    "## 対応関係": "互換性の判断方法を説明します。",
    [`## v${version}への移行`]: "配布物を入れ替えてください。",
    "## 更新とロールバック": "以前の配布物へ戻せます。",
    "## 既知の制約": "- 既知の制約です。",
    "## 配布物の検証": "checksumを検証してください。",
    ...overrides,
  };
  return `${releaseNotesTitle(product, version)}\n\n${Object.entries(bodies)
    .map(([heading, body]) => `${heading}\n\n${body}\n`)
    .join("\n")}`;
}

test("toolchain manifestはRustとNode.jsの版だけを持つ", () => {
  assert.deepEqual(Object.keys(toolchains).sort(), ["nodeVersion", "rustVersion", "schemaVersion"]);
  assert.equal(toolchains.schemaVersion, 1);
  assert.match(toolchains.rustVersion, /^\d+\.\d+\.\d+$/);
  assert.match(toolchains.nodeVersion, /^\d+\.\d+\.\d+$/);
});

test("distribution planのversionSourceから六製品の版を取得する", () => {
  assert.deepEqual(PRODUCT_IDS, ["cli", "lsp", "browser", "textlint", "vscode", "zed"]);
  for (const product of PRODUCT_IDS) {
    assert.match(productRelease(product).version, /^\d+\.\d+\.\d+$/);
  }
  assert.throws(() => productRelease("unknown"), /未知の製品/);
});

test("関連するAPI世代だけを製品ごとに返す", () => {
  assert.deepEqual(relatedApiVersions("cli"), []);
  assert.deepEqual(relatedApiVersions("lsp"), []);
  assert.deepEqual(relatedApiVersions("browser"), [
    { name: "WASM protocol schema", version: 15 },
  ]);
  assert.deepEqual(relatedApiVersions("textlint"), [{ name: "textlint adapter API", version: 1 }]);
  assert.deepEqual(relatedApiVersions("vscode"), []);
  assert.deepEqual(relatedApiVersions("zed"), []);
});

test("選択した製品と版を題名および見出しで検査する", () => {
  const { version } = productRelease("lsp");
  const source = sampleSource("lsp", version);
  const parsed = validateReleaseNotesSource(source, "lsp", version);
  assert.equal(parsed.title, `# AdocWeave lsp v${version}`);
  assert.deepEqual(parsed.sections.map(({ heading }) => heading), releaseNoteHeadings(version));
  assert.throws(() => validateReleaseNotesSource(source, "cli", version), /題名/);
  assert.throws(() => validateReleaseNotesSource(source.replace("## 既知の制約", "## 制約"), "lsp", version), /必須順序/);
  assert.throws(() => validateReleaseNotesSource(source.replace("- 変更の要点です。", "TODO"), "lsp", version), /未記入/);
});

test("対応関係には製品版と関連API世代だけを追記する", () => {
  const { version } = productRelease("lsp");
  const rendered = renderReleaseNotes(sampleSource("lsp", version), "lsp");
  const parsed = parseReleaseNotesSource(`${releaseNotesTitle("lsp", version)}\n${rendered}`);
  const compatibility = parsed.sections.find(({ heading }) => heading === "## 対応関係").lines.join("\n");
  assert.match(compatibility, new RegExp(`製品バージョン：${version.replaceAll(".", "\\.")}`));
  assert.doesNotMatch(compatibility, /LSP API/);
  assert.doesNotMatch(rendered, /統一package version|release manifest|Rust APIの破壊的変更|Rust toolchain/);
});

test("現在のRelease NotesはLSP製品のtagに対応する", () => {
  const release = productRelease("lsp");
  const tag = `${release.route.tagPrefix}${release.version}`;
  const source = loadReleaseNotesSource();
  assert.doesNotThrow(() => validateReleaseNotesSource(source, "lsp", release.version));
  const body = buildReleaseNotes("lsp", tag);
  assert.doesNotThrow(() => validateReleaseNotes(body, "lsp"));
  assert.throws(() => buildReleaseNotes("lsp", `adocweave-lsp/v9.9.9`), /専用/);
});
