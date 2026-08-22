import { readFileSync } from "node:fs";
import process from "node:process";

import { productRelease, relatedApiVersions } from "./release-policy.mjs";

const ROOT = new URL("../", import.meta.url);
export const RELEASE_NOTES_SOURCE = "release/notes.md";

export function releaseNotesTitle(product, version) {
  return `# AdocWeave ${product} v${version}`;
}

export function releaseNoteHeadings(version) {
  return [
    "## 主な変更",
    "## 対応環境",
    "## 対応関係",
    `## v${version}への移行`,
    "## 更新とロールバック",
    "## 既知の制約",
    "## 配布物の検証",
  ];
}

const UNFINISHED_MARKERS = /TODO|FIXME|記載してから公開/;

export function loadReleaseNotesSource() {
  return readFileSync(new URL(RELEASE_NOTES_SOURCE, ROOT), "utf8");
}

export function parseReleaseNotesSource(text) {
  const lines = text.split("\n");
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
  return { title: lines[0] ?? "", preamble, sections };
}

function sectionBody(section) {
  return section.lines.join("\n").trim();
}

export function validateReleaseNotesSource(text, product, version) {
  const expectedTitle = releaseNotesTitle(product, version);
  const parsed = parseReleaseNotesSource(text);
  if (parsed.title !== expectedTitle) {
    throw new Error(`Release Notesの題名が公開する製品と版に一致しません：期待「${expectedTitle}」、実際「${parsed.title}」`);
  }
  if (parsed.preamble.some((line) => line.trim().length > 0)) {
    throw new Error("Release Notesの題名と最初の見出しの間に本文を置けません");
  }
  const required = releaseNoteHeadings(version);
  const headings = parsed.sections.map(({ heading }) => heading);
  if (headings.join("\n") !== required.join("\n")) {
    throw new Error("Release Notesの見出しが製品Releaseの必須順序と一致しません");
  }
  for (const section of parsed.sections) {
    if (sectionBody(section).length === 0) {
      throw new Error(`Release Notesに本文のない節があります：${section.heading}`);
    }
  }
  if (UNFINISHED_MARKERS.test(text)) throw new Error("Release Notesに未記入の目印が残っています");
  return parsed;
}

function compatibilityLines(product, version) {
  return [
    `- 製品バージョン：${version}`,
    ...relatedApiVersions(product).map(({ name, version: apiVersion }) => `- ${name}バージョン：${apiVersion}`),
  ];
}

export function renderReleaseNotes(source, product) {
  const { version } = productRelease(product);
  const parsed = validateReleaseNotesSource(source, product, version);
  const sections = parsed.sections.map((section) => {
    const generated = section.heading === "## 対応関係"
      ? compatibilityLines(product, version).join("\n")
      : "";
    return [section.heading, sectionBody(section), generated].filter(Boolean).join("\n\n");
  });
  return `${sections.join("\n\n")}\n`;
}

export function buildReleaseNotes(product, tag) {
  const release = productRelease(product);
  const expectedTag = `${release.route.tagPrefix}${release.version}`;
  if (tag !== expectedTag) throw new Error(`Release Notesは${expectedTag}専用です`);
  return renderReleaseNotes(loadReleaseNotesSource(), product);
}

export function validateReleaseNotes(body, product) {
  const { version } = productRelease(product);
  for (const heading of releaseNoteHeadings(version)) {
    if (!body.includes(heading)) throw new Error(`Release Notesに必須見出しがありません：${heading}`);
  }
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  const [first, second] = process.argv.slice(2);
  try {
    if (first === "--check") {
      if (!second) throw new Error("検査する製品を指定してください");
      const release = productRelease(second);
      const output = buildReleaseNotes(second, `${release.route.tagPrefix}${release.version}`);
      validateReleaseNotes(output, second);
      process.stdout.write(`Release Notesの入力を確認しました：${RELEASE_NOTES_SOURCE}（${second} v${release.version}）\n`);
    } else {
      if (!first || !second) throw new Error("使用方法：node tools/release-notes.mjs PRODUCT TAG");
      for await (const _chunk of process.stdin) {
        // 呼出側のpipeを正常に終了させるため、標準入力を最後まで読みます。
      }
      const output = buildReleaseNotes(first, second);
      validateReleaseNotes(output, first);
      process.stdout.write(output);
    }
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
