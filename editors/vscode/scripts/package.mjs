import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { unzipSync, zipSync } from "fflate";

const extensionRoot = resolve(fileURLToPath(new URL("../", import.meta.url)));
const repositoryRoot = resolve(extensionRoot, "../..");
const packageJson = JSON.parse(readFileSync(join(extensionRoot, "package.json"), "utf8"));
const output = join(
  repositoryRoot,
  "target",
  "distrib",
  `adocweave-vscode-${packageJson.version}.vsix`,
);
const allowed = new Set([
  "[Content_Types].xml",
  "extension.vsixmanifest",
  "extension/LICENSE.txt",
  "extension/readme.md",
  "extension/dist/extension.cjs",
  "extension/language-configuration.json",
  "extension/package.json",
  "extension/syntaxes/asciidoc.tmLanguage.json",
]);

function normalizedPackage(scratch, suffix) {
  rmSync(join(extensionRoot, "dist"), { force: true, recursive: true });
  execFileSync("node", ["scripts/build.mjs"], { cwd: extensionRoot, stdio: "inherit" });
  const stage = join(scratch, `stage-${suffix}`);
  mkdirSync(join(stage, "dist"), { recursive: true });
  mkdirSync(join(stage, "syntaxes"), { recursive: true });
  for (const name of ["README.md", "language-configuration.json", "package.json"]) {
    copyFileSync(join(extensionRoot, name), join(stage, name));
  }
  // rust-analyzerと同じく、二重ライセンスの宣言と両原文を1つのLICENSEにまとめて同梱します。
  // 拡張registryとeditorはLICENSEという名前のfileだけをライセンスとして検出するためです。
  copyFileSync(join(extensionRoot, "LICENSE"), join(stage, "LICENSE"));
  copyFileSync(join(extensionRoot, "dist", "extension.cjs"), join(stage, "dist", "extension.cjs"));
  copyFileSync(
    join(extensionRoot, "syntaxes", "asciidoc.tmLanguage.json"),
    join(stage, "syntaxes", "asciidoc.tmLanguage.json"),
  );
  const raw = join(scratch, `raw-${suffix}.vsix`);
  execFileSync(
    join(extensionRoot, "node_modules", ".bin", "vsce"),
    ["package", "--no-dependencies", "--out", raw],
    { cwd: stage, stdio: "pipe" },
  );
  const entries = unzipSync(readFileSync(raw));
  const names = Object.keys(entries).sort();
  if (names.length !== allowed.size || names.some((name) => !allowed.has(name))) {
    throw new Error(`VSIXに予期しないfileがあります：${names.join(", ")}`);
  }
  const canonical = Object.fromEntries(names.map((name) => [name, entries[name]]));
  const entryHashes = Object.fromEntries(
    names.map((name) => [name, createHash("sha256").update(entries[name]).digest("hex")]),
  );
  return {
    bytes: zipSync(canonical, { level: 9, mtime: new Date("1980-01-01T00:00:00.000Z") }),
    entries,
    entryHashes,
  };
}

const scratch = mkdtempSync(join(tmpdir(), "adocweave-vsix-"));
try {
  const first = normalizedPackage(scratch, "a");
  const second = normalizedPackage(scratch, "b");
  if (JSON.stringify(first.entryHashes) !== JSON.stringify(second.entryHashes)) {
    throw new Error("VSIX entryのbuild結果が決定的ではありません");
  }
  const firstHash = createHash("sha256").update(first.bytes).digest("hex");
  const secondHash = createHash("sha256").update(second.bytes).digest("hex");
  if (firstHash !== secondHash) throw new Error("VSIX buildが決定的ではありません");
  if (first.bytes.byteLength > 5 * 1024 * 1024) throw new Error("VSIXが5 MiBを超えています");
  for (const forbidden of ["/workspace/", "/home/", "/tmp/", extensionRoot]) {
    for (const [name, contents] of Object.entries(first.entries)) {
      if (Buffer.from(contents).toString("utf8").includes(forbidden)) {
        throw new Error(`VSIX entry ${name}に機械固有pathが含まれています`);
      }
    }
  }
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, first.bytes);
  process.stdout.write(`決定的VSIXを作成しました：${output}\n`);
} finally {
  rmSync(scratch, { force: true, recursive: true });
}
