import { readFileSync } from "node:fs";
import process from "node:process";

import {
  PUBLIC_PROTOCOL_SCHEMA_VERSION,
  RELEASE_NOTES_VERSION,
} from "./release-policy.mjs";
import { loadBreakingRustApi } from "./breaking-rust-api.mjs";
import { loadTextlintPluginPackageContract } from "./textlint-plugin-package-contract.mjs";
import { WORKER_PROTOCOL_VERSION } from "../web-worker/worker-protocol.mjs";

// Release Notesの本文は人が``release/notes.md``へ書き、このtoolはその検証と機械生成部分の追記だけを
// 行います。版ごとに変わる説明をcodeへ書くと、Release用Pull Requestの差分から公開内容を読み取りにくく、
// 3製品で同じ手順を取れません。target一覧、契約version、Rust APIの破壊的変更と移行、textlintの
// 対応範囲は、正本のfileから生成して人の文章の後ろへ追記します。

const ROOT = new URL("../", import.meta.url);
const manifest = JSON.parse(readFileSync(new URL("release-manifest.json", ROOT), "utf8"));
const plan = JSON.parse(readFileSync(new URL("release/distribution-plan.json", ROOT), "utf8"));

const textlintContract = loadTextlintPluginPackageContract();
const breakingRustApi = loadBreakingRustApi();
export { RELEASE_NOTES_VERSION };
export const RELEASE_NOTES_PROTOCOL_SCHEMA_VERSION = PUBLIC_PROTOCOL_SCHEMA_VERSION;
export const RELEASE_NOTES_SOURCE = manifest.releaseNotes;
export const RELEASE_NOTES_TITLE = `# AdocWeave v${RELEASE_NOTES_VERSION}`;

const releaseVersionParts = RELEASE_NOTES_VERSION.split(".").map(Number);
if (releaseVersionParts.length !== 3 || releaseVersionParts.some((part) => !Number.isInteger(part))) {
  throw new Error(`Release NotesのversionがSemVerではありません：${RELEASE_NOTES_VERSION}`);
}
if (breakingRustApi.releaseVersion !== RELEASE_NOTES_VERSION) {
  throw new Error(
    `破壊的変更記録のreleaseVersionがRelease Notesと一致しません：${breakingRustApi.releaseVersion}`,
  );
}
if (typeof RELEASE_NOTES_SOURCE !== "string" || RELEASE_NOTES_SOURCE.length === 0) {
  throw new Error("release-manifest.jsonのreleaseNotesがRelease Notesのfileを指していません");
}

/// 3製品で共通の必須見出し。公開のたびに読み手が同じ観点を確認できるように、この順で要求します。
export const REQUIRED_RELEASE_NOTE_HEADINGS = [
  "## 主な変更",
  "## 対応環境",
  "## 公開契約と破壊的変更",
  `## v${RELEASE_NOTES_VERSION}への移行`,
  "## 更新とロールバック",
  "## 既知の制約",
  "## 配布物の検証",
];

/// 未記入のまま公開しないための目印。
const UNFINISHED_MARKERS = /TODO|FIXME|記載してから公開/;

export function breakingContractNotes(changes) {
  if (changes.length === 0) return ["Rust APIの破壊的変更：ありません。"];
  // descriptionは任意です。書かれていない場合はcargo-semver-checksの出力から
  // 機械的に文を作り、利用者向けの説明はrelease/notes.mdの本文へ委ねます。
  return changes.map(
    (change) =>
      `Rust APIの破壊的変更：${change.description ?? `${change.crate}の${change.item}（${change.lint}）`}`,
  );
}

export function breakingMigrationNotes(changes) {
  return changes.map((change) => change.migration).filter((migration) => migration !== undefined);
}

/// Public contracts this release states are unchanged since the previous stable tag.
///
/// The sentence in the notes is built from this list rather than written beside
/// it. v0.27.2 announced that the configuration schema had not changed while the
/// same release changed it: the claim was prose, so nothing compared it with the
/// diff. `tools/release-claims.mjs` reads this list and checks every entry that
/// has a single machine-readable source of truth.
export const UNCHANGED_CONTRACTS = [
  "CLI引数",
  "Language Server protocol",
  "設定schema",
];

/// The file that decides whether a named contract changed.
///
/// A contract without an entry here is stated but not checked: CLI arguments and
/// the Language Server protocol are spread across the sources that implement
/// them, and a file diff would report every unrelated edit. The tool names the
/// unchecked contracts in its output so the reader knows how far the check goes.
export const CONTRACT_SOURCES = {
  "WASM protocol": "web-worker/protocol.d.mts",
  設定schema: "config/adocweave.schema.json",
  "textlint Processorパッケージ契約": "release/textlint-plugin-package-contract.json",
};

/// Fields that carry the release version rather than the contract's shape.
export const CONTRACT_VERSION_FIELDS = ["packageVersion"];

function markdownList(items) {
  return items.map((item) => `- ${item}`).join("\n");
}

const MINIMUM_OS_DESCRIPTIONS = {
  "darwin:14.0": "macOS 14.0以降",
  "win32:10.0.17763": "Windows 10 version 1809（build 10.0.17763）以降",
};

function minimumOsDescription(target) {
  if (target.minimumOsVersion === null) return "";
  const description = MINIMUM_OS_DESCRIPTIONS[`${target.os}:${target.minimumOsVersion}`];
  if (!description) {
    throw new Error(`最小対応OS版の説明がありません：${target.os} ${target.minimumOsVersion}`);
  }
  return `、${description}`;
}

function targetList() {
  const osNames = { darwin: "macOS", linux: "Linux", win32: "Windows" };
  return plan.targets
    .map(
      (target) =>
        `- ${osNames[target.os]} ${target.architecture}（\`${target.triple}\`${minimumOsDescription(target)}）`,
    )
    .join("\n");
}

/// 見出しごとに、人の文章の後ろへ追記する機械生成部分。正本のfileから決まる値だけを置きます。
const GENERATED_SECTIONS = {
  "## 対応環境": () => [
    targetList(),
    `構築に使用したRust toolchainは${manifest.rustVersion}（flake.lockで固定）、Node.jsは${manifest.nodeVersion}（release-manifest.jsonで固定）です。`,
  ],
  "## 公開契約と破壊的変更": () => [
    markdownList([
      `統一package version：${RELEASE_NOTES_VERSION}`,
      `release manifest schema version：${manifest.schemaVersion}（3製品共通schema）、distribution plan schema version：${plan.schemaVersion}、配布manifest schema version：2。`,
      `WASM protocol schema version：${RELEASE_NOTES_PROTOCOL_SCHEMA_VERSION}、Worker protocol version：${WORKER_PROTOCOL_VERSION}。`,
      ...breakingContractNotes(breakingRustApi.changes),
      `${UNCHANGED_CONTRACTS.join("、")}は変更していません。`,
    ]),
  ],
  [`## v${RELEASE_NOTES_VERSION}への移行`]: () => {
    const notes = breakingMigrationNotes(breakingRustApi.changes);
    return notes.length === 0 ? [] : [markdownList(notes)];
  },
  "## 既知の制約": () => [
    `textlint用Processorの対応範囲はNode.js \`\`${textlintContract.compatibility.nodeEngine}\`\`、textlint \`\`${textlintContract.compatibility.textlintVersion}\`\`です。`,
  ],
};

/// 本文が述べるschema versionの遷移先と、正本の現在値の対応。
///
/// 過去のReleaseで、行っていないmanifestの変更を告知したことがあります。遷移を述べる文は
/// 人が書きますが、到達値は必ず正本の現在値と一致させます。
const SCHEMA_VERSION_AUTHORITIES = {
  "WASM protocol schema version": RELEASE_NOTES_PROTOCOL_SCHEMA_VERSION,
  "Worker protocol version": WORKER_PROTOCOL_VERSION,
  "release manifest schema version": manifest.schemaVersion,
  "distribution plan schema version": plan.schemaVersion,
};

export function loadReleaseNotesSource() {
  return readFileSync(new URL(RELEASE_NOTES_SOURCE, ROOT), "utf8");
}

/// ``release/notes.md``を題名と``## ``見出しの節へ分けます。
export function parseReleaseNotesSource(text) {
  const lines = text.split("\n");
  const title = lines[0] ?? "";
  const sections = [];
  const preamble = [];
  let current = null;
  for (const line of lines.slice(1)) {
    if (line.startsWith("## ")) {
      current = { heading: line, lines: [] };
      sections.push(current);
    } else if (current) {
      current.lines.push(line);
    } else {
      preamble.push(line);
    }
  }
  return { title, preamble, sections };
}

function sectionBody(section) {
  return section.lines.join("\n").trim();
}

/// 人が書いたRelease Notesが公開できる形かを検査します。
export function validateReleaseNotesSource(text, version = RELEASE_NOTES_VERSION) {
  const title = `# AdocWeave v${version}`;
  const parsed = parseReleaseNotesSource(text);
  if (parsed.title !== title) {
    throw new Error(`Release Notesの題名が公開する版と一致しません：期待「${title}」、実際「${parsed.title}」`);
  }
  if (parsed.preamble.some((line) => line.trim().length > 0)) {
    throw new Error("Release Notesの題名と最初の見出しの間に本文を置けません");
  }
  const required = REQUIRED_RELEASE_NOTE_HEADINGS.map((heading) =>
    heading.replace(`v${RELEASE_NOTES_VERSION}`, `v${version}`),
  );
  const headings = parsed.sections.map((section) => section.heading);
  for (const heading of required) {
    const count = headings.filter((candidate) => candidate === heading).length;
    if (count !== 1) throw new Error(`Release Notesに必須の見出しが一つだけ現れていません：${heading}`);
  }
  for (const heading of headings) {
    if (!required.includes(heading)) throw new Error(`Release Notesに共通見出し以外の節があります：${heading}`);
  }
  if (headings.join("\n") !== required.join("\n")) {
    throw new Error("Release Notesの見出しの順序が共通の順序と一致しません");
  }
  for (const section of parsed.sections) {
    if (sectionBody(section).length === 0) {
      throw new Error(`Release Notesに本文のない節があります：${section.heading}`);
    }
  }
  if (UNFINISHED_MARKERS.test(text)) {
    throw new Error("Release Notesに未記入の目印が残っています");
  }
  for (const [, name, , to] of text.matchAll(/([A-Za-z ]*(?:schema|protocol) version)を(\d+)から(\d+)へ/g)) {
    const authority = SCHEMA_VERSION_AUTHORITIES[name.trim()];
    if (authority === undefined) {
      throw new Error(`Release Notesが述べる「${name.trim()}」の正本が登録されていません`);
    }
    if (Number(to) !== authority) {
      throw new Error(`Release Notesが述べる${name.trim()}の遷移先${to}が現在値${authority}と一致しません`);
    }
  }
  return parsed;
}

/// 人の文章の各節の後ろへ機械生成部分を追記した、公開するRelease Notesの本文を返します。
/// 題名はGitHub ReleaseのnameにあるためBodyへは含めません。
export function renderReleaseNotes(source) {
  const parsed = validateReleaseNotesSource(source);
  const sections = parsed.sections.map((section) => {
    const generated = (GENERATED_SECTIONS[section.heading] ?? (() => []))();
    return [section.heading, sectionBody(section), ...generated].join("\n\n");
  });
  return `${sections.join("\n\n")}\n`;
}

export function buildReleaseNotes(tag) {
  if (tag !== `v${RELEASE_NOTES_VERSION}`) {
    throw new Error(`Release Notesはv${RELEASE_NOTES_VERSION}専用です`);
  }
  return renderReleaseNotes(loadReleaseNotesSource());
}

export function validateReleaseNotes(body) {
  for (const heading of REQUIRED_RELEASE_NOTE_HEADINGS) {
    if (!body.includes(heading)) throw new Error(`Release Notesに必須見出しがありません：${heading}`);
  }
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  const argument = process.argv[2];
  if (argument === "--check") {
    // release-contract taskから呼び、公開前にrelease/notes.mdの形と機械生成部分を確認します。
    const output = buildReleaseNotes(`v${RELEASE_NOTES_VERSION}`);
    validateReleaseNotes(output);
    process.stdout.write(`Release Notesの入力を確認しました：${RELEASE_NOTES_SOURCE}（v${RELEASE_NOTES_VERSION}）\n`);
  } else {
    // cargo-distの自動生成本文は英語であるため読み捨て、単一の日本語本文を生成します。
    for await (const _chunk of process.stdin) {
      // 標準入力を最後まで読み、呼出側のpipeを正常に終了させます。
    }
    const output = buildReleaseNotes(argument);
    validateReleaseNotes(output);
    process.stdout.write(output);
  }
}
