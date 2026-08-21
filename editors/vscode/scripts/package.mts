import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { type Unzipped, unzipSync, zipSync } from "fflate";

import { type ExtensionManifest, readJson } from "./manifests.mts";

const extensionRoot = resolve(fileURLToPath(new URL("../", import.meta.url)));
const repositoryRoot = resolve(extensionRoot, "../..");
const packageJson = readJson<ExtensionManifest>(join(extensionRoot, "package.json"));
const output = join(
  repositoryRoot,
  "target",
  "distrib",
  `adocweave-vscode-${packageJson.version}.vsix`,
);
/** Every entry the VSIX may contain; anything else fails the build. */
const allowed: ReadonlySet<string> = new Set([
  "[Content_Types].xml",
  "extension.vsixmanifest",
  "extension/LICENSE-APACHE",
  "extension/LICENSE-MIT",
  "extension/README.adoc",
  "extension/dist/extension.cjs",
  "extension/language-configuration.json",
  "extension/package.json",
  "extension/syntaxes/asciidoc.tmLanguage.json",
]);
const maximumBytes = 5 * 1024 * 1024;

interface NormalizedPackage {
  bytes: Uint8Array;
  entries: Unzipped;
  entryHashes: Record<string, string>;
}

function sha256(bytes: Uint8Array): string {
  return createHash("sha256").update(bytes).digest("hex");
}

/**
 * Builds the extension, packages it with vsce from a staged copy of the
 * allowed files, and re-zips the entries with a fixed timestamp so the bytes
 * depend only on the inputs.
 */
function normalizedPackage(scratch: string, suffix: string): NormalizedPackage {
  rmSync(join(extensionRoot, "dist"), { force: true, recursive: true });
  execFileSync("node", ["scripts/build.mts"], { cwd: extensionRoot, stdio: "inherit" });
  const stage = join(scratch, `stage-${suffix}`);
  mkdirSync(join(stage, "dist"), { recursive: true });
  mkdirSync(join(stage, "syntaxes"), { recursive: true });
  for (const name of ["README.adoc", "language-configuration.json", "package.json"]) {
    copyFileSync(join(extensionRoot, name), join(stage, name));
  }
  copyFileSync(join(repositoryRoot, "LICENSE-APACHE"), join(stage, "LICENSE-APACHE"));
  copyFileSync(join(repositoryRoot, "LICENSE-MIT"), join(stage, "LICENSE-MIT"));
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
    throw new Error(`Unexpected files in the VSIX: ${names.join(", ")}`);
  }
  const canonical: Unzipped = {};
  const entryHashes: Record<string, string> = {};
  for (const name of names) {
    const contents = entries[name];
    if (contents === undefined) throw new Error(`Missing VSIX entry: ${name}`);
    canonical[name] = contents;
    entryHashes[name] = sha256(contents);
  }
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
    throw new Error("The VSIX entries are not built deterministically");
  }
  if (sha256(first.bytes) !== sha256(second.bytes)) {
    throw new Error("The VSIX is not built deterministically");
  }
  if (first.bytes.byteLength > maximumBytes) throw new Error("The VSIX exceeds 5 MiB");
  for (const forbidden of ["/workspace/", "/home/", "/tmp/", extensionRoot]) {
    for (const [name, contents] of Object.entries(first.entries)) {
      if (Buffer.from(contents).toString("utf8").includes(forbidden)) {
        throw new Error(`VSIX entry ${name} contains a machine-specific path`);
      }
    }
  }
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, first.bytes);
  process.stdout.write(`Created a deterministic VSIX: ${output}\n`);
} finally {
  rmSync(scratch, { force: true, recursive: true });
}
