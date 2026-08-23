import { readFileSync } from "node:fs";
import process from "node:process";

import { productRelease, relatedApiVersions } from "./release-policy.mjs";

const ROOT = new URL("../", import.meta.url);
export const RELEASE_NOTES_SOURCE = "release/notes.md";
export const RELEASE_NOTES_TEMPLATE_SOURCE = "release/notes.template.md";
const RELEASE_NOTES_TITLE =
  /^# AdocWeave ([a-z0-9-]+) v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

export function releaseNotesTitle(product, version) {
  const title = releaseNotesTemplateStructure().title;
  if (title.split("PRODUCT").length !== 2 || title.split("X.Y.Z").length !== 2) {
    throw new Error("Release Notesの雛形の題名はPRODUCTとX.Y.Zを1回ずつ含める必要があります");
  }
  return title.replace("PRODUCT", product).replace("X.Y.Z", version);
}

export function releaseNoteHeadings(version) {
  return releaseNotesTemplateStructure().headings.map((heading) =>
    heading.replaceAll("X.Y.Z", version));
}

const UNFINISHED_MARKERS = /TODO|FIXME|記載してから公開/;

export function loadReleaseNotesSource() {
  return readFileSync(new URL(RELEASE_NOTES_SOURCE, ROOT), "utf8");
}

export function loadReleaseNotesTemplateSource() {
  return readFileSync(new URL(RELEASE_NOTES_TEMPLATE_SOURCE, ROOT), "utf8");
}

function releaseNotesTemplateStructure() {
  const lines = loadReleaseNotesTemplateSource().split("\n");
  const title = lines[0] ?? "";
  const headings = lines.filter((line) => line.startsWith("## "));
  if (headings.length === 0 || new Set(headings).size !== headings.length) {
    throw new Error("Release Notesの雛形は重複のない見出しを含める必要があります");
  }
  return { title, headings };
}

export function releaseNotesSelection(source) {
  const title = source.split("\n", 1)[0] ?? "";
  const match = RELEASE_NOTES_TITLE.exec(title);
  if (!match) {
    throw new Error("Release Notesの題名から公開する製品と版を判定できません");
  }
  const product = match[1];
  const version = `${match[2]}.${match[3]}.${match[4]}`;
  const release = productRelease(product);
  if (version !== release.version) {
    throw new Error(
      `Release Notesの版が${product}の公開版と一致しません：期待${release.version}、実際${version}`,
    );
  }
  return { product, version };
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

export function parseReleaseNotesArguments(args) {
  if (args.length === 1 && args[0] === "--check") {
    return { mode: "check", product: undefined, tag: undefined };
  }
  if (args.length === 2 && args[0] === "--check") {
    return { mode: "check", product: args[1], tag: undefined };
  }
  if (args.length === 2 && args[0] !== "--check") {
    return { mode: "render", product: args[0], tag: args[1] };
  }
  throw new Error(
    "使用方法：node tools/release-notes.mjs --check [PRODUCT] | node tools/release-notes.mjs PRODUCT TAG",
  );
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    const command = parseReleaseNotesArguments(process.argv.slice(2));
    if (command.mode === "check") {
      const source = loadReleaseNotesSource();
      const product = command.product ?? releaseNotesSelection(source).product;
      const release = productRelease(product);
      const output = buildReleaseNotes(product, `${release.route.tagPrefix}${release.version}`);
      validateReleaseNotes(output, product);
      process.stdout.write(`Release Notesの入力を確認しました：${RELEASE_NOTES_SOURCE}（${product} v${release.version}）\n`);
    } else {
      for await (const _chunk of process.stdin) {
        // 呼出側のpipeを正常に終了させるため、標準入力を最後まで読みます。
      }
      const output = buildReleaseNotes(command.product, command.tag);
      validateReleaseNotes(output, command.product);
      process.stdout.write(output);
    }
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
