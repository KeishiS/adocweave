import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  RELEASE_NOTES_PROTOCOL_SCHEMA_VERSION,
  RELEASE_NOTES_SOURCE,
  RELEASE_NOTES_TITLE,
  RELEASE_NOTES_VERSION,
  REQUIRED_RELEASE_NOTE_HEADINGS,
  breakingContractNotes,
  breakingMigrationNotes,
  buildReleaseNotes,
  loadReleaseNotesSource,
  parseReleaseNotesSource,
  renderReleaseNotes,
  validateReleaseNotes,
  validateReleaseNotesSource,
} from "./release-notes.mjs";
import manifest from "../release-manifest.json" with { type: "json" };
import protocol from "../protocol/public-api.json" with { type: "json" };
import { loadTextlintPluginPackageContract } from "./textlint-plugin-package-contract.mjs";

const textlintContract = loadTextlintPluginPackageContract();
const escape = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

function sampleSource(version = RELEASE_NOTES_VERSION, overrides = {}) {
  const bodies = {
    "## 主な変更": "- 変更の要点です。",
    "## 対応環境": "配布するtargetは次のとおりです。",
    "## 公開契約と破壊的変更": "公開契約は変更していません。",
    [`## v${version}への移行`]: "配布物を入れ替えてください。",
    "## 更新とロールバック": "以前のdirectoryへ戻します。",
    "## 既知の制約": "- 既知の制約です。",
    "## 配布物の検証": "checksumとattestationを検証してください。",
    ...overrides,
  };
  return `# AdocWeave v${version}\n\n${
    Object.entries(bodies).map(([heading, body]) => `${heading}\n\n${body}\n`).join("\n")
  }`;
}

test("Release Notesの人が書く部分はrelease/notes.mdであり、契約値をcodeへ直書きしない", () => {
  assert.equal(RELEASE_NOTES_SOURCE, "release/notes.md");
  assert.equal(manifest.releaseNotes, RELEASE_NOTES_SOURCE);
  const source = readFileSync(new URL("./release-notes.mjs", import.meta.url), "utf8");
  assert.match(source, /loadTextlintPluginPackageContract/);
  for (const value of [
    textlintContract.identity.packageName,
    textlintContract.identity.pluginName,
    textlintContract.compatibility.nodeEngine,
    textlintContract.compatibility.textlintVersion,
  ]) {
    assert.equal(source.includes(value), false, `Release Notesに契約値を直書きしています：${value}`);
  }
  const notes = loadReleaseNotesSource();
  for (const value of [
    textlintContract.compatibility.nodeEngine,
    textlintContract.compatibility.textlintVersion,
    manifest.rustVersion,
  ]) {
    assert.equal(notes.includes(value), false, `release/notes.mdに機械生成する値を直書きしています：${value}`);
  }
});

test("必須見出しは3製品共通の7見出しをこの順で要求する", () => {
  assert.deepEqual(REQUIRED_RELEASE_NOTE_HEADINGS, [
    "## 主な変更",
    "## 対応環境",
    "## 公開契約と破壊的変更",
    `## v${RELEASE_NOTES_VERSION}への移行`,
    "## 更新とロールバック",
    "## 既知の制約",
    "## 配布物の検証",
  ]);
  assert.equal(RELEASE_NOTES_TITLE, `# AdocWeave v${RELEASE_NOTES_VERSION}`);
});

test("release/notes.mdは題名、見出しの順序、本文の有無を満たす", () => {
  const source = loadReleaseNotesSource();
  const parsed = validateReleaseNotesSource(source);
  assert.equal(parsed.title, RELEASE_NOTES_TITLE);
  assert.deepEqual(parsed.sections.map((section) => section.heading), REQUIRED_RELEASE_NOTE_HEADINGS);
});

test("人が書いた入力の不備を名指しで拒否する", () => {
  assert.doesNotThrow(() => validateReleaseNotesSource(sampleSource()));
  assert.throws(() => validateReleaseNotesSource(sampleSource("9.9.9")), /題名が公開する版と一致しません/);
  assert.throws(
    () => validateReleaseNotesSource(sampleSource().replace("## 既知の制約\n", "## 制約\n")),
    /必須の見出しが一つだけ現れていません：## 既知の制約/,
  );
  assert.throws(
    () => validateReleaseNotesSource(sampleSource().replace("## 既知の制約\n\n- 既知の制約です。\n", "## 既知の制約\n")),
    /本文のない節があります：## 既知の制約/,
  );
  assert.throws(
    () => validateReleaseNotesSource(`${sampleSource()}\n## 謝辞\n\nありがとうございます。\n`),
    /共通見出し以外の節があります：## 謝辞/,
  );
  const swapped = sampleSource()
    .replace("## 主な変更\n\n- 変更の要点です。\n", "")
    .replace("## 配布物の検証", "## 主な変更\n\n- 変更の要点です。\n\n## 配布物の検証");
  assert.throws(() => validateReleaseNotesSource(swapped), /見出しの順序が共通の順序と一致しません/);
  assert.throws(
    () => validateReleaseNotesSource(sampleSource(RELEASE_NOTES_VERSION, { "## 主な変更": "TODO 後で書く" })),
    /未記入の目印/,
  );
  assert.throws(
    () => validateReleaseNotesSource(`# AdocWeave v${RELEASE_NOTES_VERSION}\n\n前書き\n${sampleSource().slice(RELEASE_NOTES_TITLE.length)}`),
    /題名と最初の見出しの間に本文を置けません/,
  );
});

test("本文が述べるschema versionの遷移先は正本の現在値と一致しなければならない", () => {
  // 過去のReleaseで、行っていないmanifestの変更を告知したことがあります。遷移を述べる文は人が
  // 書きますが、到達値は必ず正本の現在値と一致させ、正本のない名前の遷移は受理しません。
  const transition = (name, to) =>
    sampleSource(RELEASE_NOTES_VERSION, { "## 公開契約と破壊的変更": `${name}を1から${to}へ更新しました。` });
  assert.doesNotThrow(() =>
    validateReleaseNotesSource(transition("WASM protocol schema version", RELEASE_NOTES_PROTOCOL_SCHEMA_VERSION))
  );
  assert.throws(
    () => validateReleaseNotesSource(transition("WASM protocol schema version", RELEASE_NOTES_PROTOCOL_SCHEMA_VERSION + 1)),
    /遷移先.*が現在値.*と一致しません/,
  );
  assert.doesNotThrow(() =>
    validateReleaseNotesSource(transition("release manifest schema version", manifest.schemaVersion))
  );
  assert.throws(() => validateReleaseNotesSource(transition("config schema version", 3)), /正本が登録されていません/);
});

test(`Release Notesはv${RELEASE_NOTES_VERSION}の人の文章と機械生成部分を含む`, () => {
  const notes = buildReleaseNotes(`v${RELEASE_NOTES_VERSION}`);
  assert.doesNotThrow(() => validateReleaseNotes(notes));
  assert.equal(notes.startsWith("## 主な変更"), true, "GitHub Releaseのbodyへ題名を含めません");
  // 人の文章
  assert.match(notes, /バージョンの異なる配布物を混ぜて使えない/);
  assert.match(notes, /sha256sum --check/);
  assert.match(notes, /gh attestation verify/);
  assert.match(notes, /以前のVSIXとnative directoryを保持/);
  // 公開経路の主張。v0.44.0でVS Code拡張だけがOpen VSXへ加わったため、
  // 「registryへ公開しない」とだけ述べる本文は正しくありません。
  assert.match(notes, /registryへpackageを公開せず/);
  assert.match(notes, /Open VSX/);
  // 配布計画から生成するtarget
  assert.match(notes, /x86_64-unknown-linux-musl/);
  assert.match(notes, /aarch64-apple-darwin/);
  assert.match(notes, /x86_64-pc-windows-msvc/);
  assert.match(notes, /macOS 14\.0以降/);
  assert.match(notes, /Windows 10 version 1809（build 10\.0\.17763）以降/);
  // manifest、protocol、契約から生成する値
  assert.match(notes, new RegExp(`統一package version：${escape(RELEASE_NOTES_VERSION)}`));
  assert.match(notes, new RegExp(`release manifest schema version：${manifest.schemaVersion}（3製品共通schema）`));
  assert.match(notes, new RegExp(`WASM protocol schema version：${RELEASE_NOTES_PROTOCOL_SCHEMA_VERSION}`));
  assert.match(notes, new RegExp(`Worker protocol version：${protocol.workerProtocolVersion}`));
  assert.match(notes, new RegExp(`Rust toolchainは${escape(manifest.rustVersion)}`));
  assert.match(notes, new RegExp(`Node\\.jsは${escape(manifest.nodeVersion)}`));
  assert.match(notes, /Rust APIの破壊的変更：/);
  assert.match(notes, /は変更していません。/);
  assert.match(notes, new RegExp(escape(textlintContract.compatibility.nodeEngine)));
  assert.match(notes, new RegExp(escape(textlintContract.compatibility.textlintVersion)));
  assert.match(notes, new RegExp(`## v${escape(RELEASE_NOTES_VERSION)}への移行`));

  const requestFields = protocol.request.fields.map((field) => field.json);
  assert.equal(requestFields.includes("schemaVersion"), false);
  assert.equal(requestFields.includes("packageVersion"), true);
  assert.equal(protocol.request.unknownFields, "reject");
});

test("機械生成部分は人の文章の後ろへ、見出しの順序を保って追記する", () => {
  const rendered = renderReleaseNotes(sampleSource());
  const parsed = parseReleaseNotesSource(`# AdocWeave v${RELEASE_NOTES_VERSION}\n${rendered}`);
  assert.deepEqual(parsed.sections.map((section) => section.heading), REQUIRED_RELEASE_NOTE_HEADINGS);
  const environment = parsed.sections[1].lines.join("\n");
  assert.ok(environment.indexOf("配布するtargetは次のとおりです。") < environment.indexOf("x86_64-unknown-linux-musl"));
  const contracts = parsed.sections[2].lines.join("\n");
  assert.ok(contracts.indexOf("公開契約は変更していません。") < contracts.indexOf("統一package version"));
  const constraints = parsed.sections[5].lines.join("\n");
  assert.ok(constraints.indexOf("- 既知の制約です。") < constraints.indexOf("textlint用Processorの対応範囲"));
});

test("破壊的変更が無いreleaseでは定型文を記録から生成する", () => {
  assert.deepEqual(breakingContractNotes([]), ["Rust APIの破壊的変更：ありません。"]);
  assert.deepEqual(breakingMigrationNotes([]), []);
});

test("Release Notesは別release trainのtagを拒否する", () => {
  assert.equal(manifest.packageVersion, RELEASE_NOTES_VERSION);
  const expectedError = new RegExp(`v${escape(RELEASE_NOTES_VERSION)}専用`);
  assert.throws(() => buildReleaseNotes("v9.9.9"), expectedError);
  assert.throws(() => validateReleaseNotes("Generated changes"), /必須見出し/);
});
