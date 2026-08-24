import assert from "node:assert/strict";
import test from "node:test";

import {
  buildReleaseNotes,
  loadReleaseNotesTemplateSource,
  loadReleaseNotesSource,
  parseReleaseNotesArguments,
  parseReleaseNotesSource,
  releaseNoteHeadings,
  releaseNotesSelection,
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

test("Release Notesの題名と見出しを雛形から構成する", () => {
  const template = loadReleaseNotesTemplateSource();
  const parsed = parseReleaseNotesSource(template);
  assert.equal(parsed.title, "# AdocWeave PRODUCT vX.Y.Z");
  assert.deepEqual(parsed.sections.map(({ heading }) => heading), releaseNoteHeadings("X.Y.Z"));
  assert.equal(releaseNotesTitle("cli", "1.2.3"), "# AdocWeave cli v1.2.3");
});

test("distribution planのversionSourceから六製品の版を取得する", () => {
  assert.deepEqual(PRODUCT_IDS, ["lib", "cli", "lsp", "wasm", "textlint", "vscode", "zed"]);
  for (const product of PRODUCT_IDS) {
    assert.match(productRelease(product).version, /^\d+\.\d+\.\d+$/);
  }
  assert.throws(() => productRelease("unknown"), /未知の製品/);
});

test("関連するAPI世代だけを製品ごとに返す", () => {
  assert.deepEqual(relatedApiVersions("cli"), []);
  assert.deepEqual(relatedApiVersions("lsp"), []);
  assert.deepEqual(relatedApiVersions("wasm"), [
    { name: "WASM protocol schema", version: 15 },
  ]);
  assert.deepEqual(relatedApiVersions("textlint"), []);
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

test("v0.46.2相当のLSP変更説明へ未検査の不変文を追加しない", () => {
  const { version } = productRelease("lsp");
  const rendered = renderReleaseNotes(sampleSource("lsp", version, {
    "## 主な変更":
      "- workspace-scan-incomplete診断を廃止し、window/showMessageへ移します。",
    "## 対応関係":
      "Language Server protocolの通知方法を変更するため、client側の対応が必要です。",
    [`## v${version}への移行`]:
      "診断の処理をwindow/showMessageの受信へ切り替えてください。",
  }), "lsp");

  assert.match(rendered, /workspace-scan-incomplete/);
  assert.match(rendered, /window\/showMessage/);
  assert.doesNotMatch(
    rendered,
    /(?:CLI引数|Language Server protocol|設定schema)は変更していません/,
  );
});

test("現在のRelease Notesは題名で選択した製品のtagに対応する", () => {
  const source = loadReleaseNotesSource();
  const { product } = releaseNotesSelection(source);
  const release = productRelease(product);
  const tag = `${release.route.tagPrefix}${release.version}`;
  assert.doesNotThrow(() => validateReleaseNotesSource(source, product, release.version));
  const body = buildReleaseNotes(product, tag);
  assert.doesNotThrow(() => validateReleaseNotes(body, product));
  assert.throws(() => buildReleaseNotes(product, `${release.route.tagPrefix}9.9.9`), /専用/);
});

test("Release Notesの題名は登録済み製品の現在版を指定する", () => {
  const { version } = productRelease("lsp");
  assert.deepEqual(
    releaseNotesSelection(sampleSource("lsp", version)),
    { product: "lsp", version },
  );
  assert.throws(() => releaseNotesSelection("# Release Notes\n"), /判定できません/);
  assert.throws(() => releaseNotesSelection(sampleSource("unknown", version)), /未知の製品/);
  assert.throws(() => releaseNotesSelection(sampleSource("lsp", "9.9.9")), /公開版と一致しません/);
});

test("Release NotesのCLIは検査と生成の引数を厳密に区別する", () => {
  assert.deepEqual(parseReleaseNotesArguments(["--check"]), {
    mode: "check", product: undefined, tag: undefined,
  });
  assert.deepEqual(parseReleaseNotesArguments(["--check", "lsp"]), {
    mode: "check", product: "lsp", tag: undefined,
  });
  assert.deepEqual(parseReleaseNotesArguments(["lsp", "adocweave-lsp/v0.47.0"]), {
    mode: "render", product: "lsp", tag: "adocweave-lsp/v0.47.0",
  });
  for (const args of [[], ["--check", "lsp", "extra"], ["lsp"], ["lsp", "tag", "extra"]]) {
    assert.throws(() => parseReleaseNotesArguments(args), /使用方法/);
  }
});
