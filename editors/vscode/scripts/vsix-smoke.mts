import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

import { unzipSync } from "fflate";

import { type ExtensionManifest, readJson } from "./manifests.mts";

const packageJson = readJson<ExtensionManifest>("package.json");
const path = resolve("../../target/distrib", `adocweave-vscode-${packageJson.version}.vsix`);
if (!existsSync(path)) throw new Error("The VSIX does not exist; run `npm run package` first");
const bytes = readFileSync(path);
const entries = unzipSync(bytes);
const manifestEntry = entries["extension/package.json"];
if (manifestEntry === undefined)
  throw new Error("The VSIX does not contain extension/package.json");
const manifest = JSON.parse(Buffer.from(manifestEntry).toString("utf8")) as ExtensionManifest;
if (manifest.version !== packageJson.version || manifest.main !== "./dist/extension.cjs") {
  throw new Error("The VSIX manifest does not match the source package");
}
if (!entries["extension/LICENSE-APACHE"] || !entries["extension/LICENSE-MIT"]) {
  throw new Error("The VSIX does not contain the license files");
}
if (Object.keys(entries).some((name) => name.includes("node_modules") || name.endsWith(".map"))) {
  throw new Error("The VSIX contains a dependency or source map that is not allowed");
}
process.stdout.write(
  `VSIX smoke test passed: ${bytes.byteLength} bytes, SHA-256 ${createHash("sha256").update(bytes).digest("hex")}\n`,
);
