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
      manifest.private !== undefined || !Array.isArray(manifest.files)) {
    throw new Error("textlint plugin package.jsonを解釈できません");
  }
  // scoped packageの既定はprivate公開のため、公開範囲をmanifestで明示する。
  if (manifest.publishConfig?.access !== "public") {
    throw new Error("textlint plugin package.jsonへpublishConfig.accessをpublicで指定してください");
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

export const TEXTLINT_CONSUMER_MANIFEST_URL = new URL(
  "./textlint-plugin-e2e/package.json",
  import.meta.url,
);

// packageは範囲でtextlintを受け入れるが、継続的な検査は一つの組合せで行う。
// 固定consumerの依存を、その検査対象versionの正本とする。
export function verifiedTextlintVersion(path = TEXTLINT_CONSUMER_MANIFEST_URL) {
  const version = JSON.parse(readFileSync(path, "utf8")).dependencies?.textlint;
  if (typeof version !== "string" || !/^\d+\.\d+\.\d+$/u.test(version)) {
    throw new Error("固定consumerのtextlint versionを解釈できません");
  }
  return version;
}

function versionOrder(left, right) {
  const parsed = (value) => value.split(".").map(Number);
  const [leftParts, rightParts] = [parsed(left), parsed(right)];
  for (let index = 0; index < 3; index += 1) {
    if (leftParts[index] !== rightParts[index]) return leftParts[index] - rightParts[index];
  }
  return 0;
}

// peerDependenciesで使う``^X.Y.Z``と完全一致だけを解釈する。前release版は扱わない。
export function satisfiesPeerRange(version, range) {
  if (!range.startsWith("^")) return version === range;
  const base = range.slice(1);
  const nextMajor = `${Number(base.split(".")[0]) + 1}.0.0`;
  return versionOrder(version, base) >= 0 && versionOrder(version, nextMajor) < 0;
}

export function textlintPluginName(packageName) {
  const match = /^(@[^/]+)\/textlint-plugin-(.+)$/.exec(packageName);
  if (!match) throw new Error(`textlint plugin package名が不正です: ${packageName}`);
  return `${match[1]}/${match[2]}`;
}
