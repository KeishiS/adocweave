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
  "extension/LICENSE.txt",
  "extension/readme.md",
  "extension/dist/extension.cjs",
  "extension/language-configuration.json",
  "extension/package.json",
  "extension/resources/icon.png",
  "extension/syntaxes/asciidoc.tmLanguage.json",
]);
const maximumBytes = 5 * 1024 * 1024;
// Building twice doubles the packaging time, so only the paths that produce the
// published artifact pay for it. Pull requests build once and verify the
// contents; the release artifact build compares two builds byte for byte.
const verifyDeterminism = process.argv.includes("--verify-determinism");

interface NormalizedPackage {
  bytes: Uint8Array;
  entries: Unzipped;
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
  mkdirSync(join(stage, "resources"), { recursive: true });
  mkdirSync(join(stage, "syntaxes"), { recursive: true });
  for (const name of ["README.md", "language-configuration.json", "package.json"]) {
    copyFileSync(join(extensionRoot, name), join(stage, name));
  }
  // Like rust-analyzer, ship one LICENSE that states the dual license and carries
  // both texts: extension registries and editors only detect a file named LICENSE.
  copyFileSync(join(extensionRoot, "LICENSE"), join(stage, "LICENSE"));
  copyFileSync(join(extensionRoot, "dist", "extension.cjs"), join(stage, "dist", "extension.cjs"));
  copyFileSync(join(extensionRoot, "resources", "icon.png"), join(stage, "resources", "icon.png"));
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
  for (const name of names) {
    const contents = entries[name];
    if (contents === undefined) throw new Error(`Missing VSIX entry: ${name}`);
    canonical[name] = contents;
  }
  return {
    bytes: zipSync(canonical, { level: 9, mtime: new Date("1980-01-01T00:00:00.000Z") }),
    entries,
  };
}

const scratch = mkdtempSync(join(tmpdir(), "adocweave-vsix-"));
try {
  const built = normalizedPackage(scratch, "a");
  const hash = sha256(built.bytes);
  if (verifyDeterminism && sha256(normalizedPackage(scratch, "b").bytes) !== hash) {
    throw new Error("The VSIX is not built deterministically");
  }
  if (built.bytes.byteLength > maximumBytes) throw new Error("The VSIX exceeds 5 MiB");
  for (const forbidden of ["/workspace/", "/home/", "/tmp/", extensionRoot]) {
    for (const [name, contents] of Object.entries(built.entries)) {
      if (Buffer.from(contents).toString("utf8").includes(forbidden)) {
        throw new Error(`VSIX entry ${name} contains a machine-specific path`);
      }
    }
  }
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, built.bytes);
  process.stdout.write(
    `Packaged ${output}: ${built.bytes.byteLength} bytes, SHA-256 ${hash}${
      verifyDeterminism ? ", reproduced from a second build" : ""
    }\n`,
  );
} finally {
  rmSync(scratch, { force: true, recursive: true });
}
