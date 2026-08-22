import { readFileSync } from "node:fs";

export const TEXTLINT_PLUGIN_MANIFEST_URL = new URL(
  "../packages/textlint-plugin-asciidoc/package.json",
  import.meta.url,
);

export const TEXTLINT_PLUGIN_PACKAGE_LIMITS = Object.freeze({
  maximumMemoryBytes: 256 * 1024 * 1024,
  maximumPackedBytes: 8 * 1024 * 1024,
  maximumUnpackedBytes: 16 * 1024 * 1024,
});

export const TEXTLINT_PLUGIN_WASM_PATHS = Object.freeze({
  binary: "wasm/adocweave_textlint_wasm_bg.wasm",
  wrapper: "wasm/adocweave_textlint_wasm.cjs",
});

export function loadTextlintPluginManifest(path = TEXTLINT_PLUGIN_MANIFEST_URL) {
  const manifest = JSON.parse(readFileSync(path, "utf8"));
  validateTextlintPluginManifest(manifest);
  return manifest;
}

export function validateTextlintPluginManifest(manifest) {
  if (manifest.name !== "@adocweave/textlint-plugin-asciidoc" ||
      manifest.private !== true || !Array.isArray(manifest.files)) {
    throw new Error("textlint plugin package.jsonを解釈できません");
  }
  const portable = new Set();
  for (const file of manifest.files) {
    if (typeof file !== "string") {
      throw new Error(`textlint plugin package.jsonのfile pathが不正です: ${String(file)}`);
    }
    const parts = file.split("/");
    const key = file.normalize("NFC").toLocaleLowerCase("en-US");
    if (file.startsWith("/") || /^[A-Za-z]:/.test(file) || file.includes("\\") || file.includes("\0") ||
        parts.some((part) => part === "" || part === "." || part === "..") ||
        file.normalize("NFC") !== file || portable.has(key) || file === "package.json") {
      throw new Error(`textlint plugin package.jsonのfile pathが不正です: ${String(file)}`);
    }
    portable.add(key);
  }
  if (manifest.files.join("\n") !== [...manifest.files].sort().join("\n")) {
    throw new Error("textlint plugin package.jsonのfilesはpath順に並べてください");
  }
}

export function textlintPluginName(packageName) {
  const match = /^(@[^/]+)\/textlint-plugin-(.+)$/.exec(packageName);
  if (!match) throw new Error(`textlint plugin package名が不正です: ${packageName}`);
  return `${match[1]}/${match[2]}`;
}
