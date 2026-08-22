import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

import { runTextlintPluginNpxSmoke } from "./textlint-plugin-npx-smoke.mjs";
import { loadPackageManifest } from "./textlint-plugin-npx-smoke.mjs";

const DEFAULT_MANIFEST = new URL("../packages/textlint-plugin-asciidoc/package.json", import.meta.url);

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  const manifest = JSON.parse(await readFile(DEFAULT_MANIFEST, "utf8"));
  const releaseBaseUrl = textlintReleaseBaseUrl(manifest.version);
  await runTextlintPluginPostReleaseSmoke(releaseBaseUrl);
  process.stdout.write(`textlint plugin post-release smoke passed: v${manifest.version}\n`);
}

export async function runTextlintPluginPostReleaseSmoke(
  releaseBaseUrl,
  {
    manifest,
    fetchAsset = fetchBytes,
    runNpxSmoke = runTextlintPluginNpxSmoke,
    version,
  } = {},
) {
  manifest ??= await loadPackageManifest();
  version ??= JSON.parse(await readFile(DEFAULT_MANIFEST, "utf8")).version;
  const base = new URL(`${releaseBaseUrl.replace(/\/$/, "")}/`);
  if (base.protocol !== "https:" || base.hostname !== "github.com") {
    throw new Error("post-release smoke requires an HTTPS GitHub Release URL");
  }
  if (typeof version !== "string") {
    throw new Error("textlint package manifest is missing version");
  }
  const archiveName = `adocweave-textlint-plugin-asciidoc-${version}.tgz`;
  const archiveUrl = new URL(archiveName, base).href;
  const checksumUrl = new URL("sha256.sum", base).href;
  const [archiveBytes, checksumBytes] = await Promise.all([
    fetchAsset(archiveUrl),
    fetchAsset(checksumUrl),
  ]);
  const expected = checksumFor(checksumBytes.toString("utf8"), archiveName);
  const actual = createHash("sha256").update(archiveBytes).digest("hex");
  if (actual !== expected) {
    throw new Error(`published textlint plugin checksum mismatch: ${actual} != ${expected}`);
  }

  await runNpxSmoke(archiveUrl, { manifest });
}

export function textlintReleaseBaseUrl(version) {
  return `https://github.com/KeishiS/adocweave/releases/download/adocweave-textlint%2Fv${version}`;
}

export function checksumFor(source, archiveName) {
  const matches = source.split(/\r?\n/).flatMap((line) => {
    const match = line.match(/^([0-9a-f]{64})\s+\*?(.+)$/);
    return match && match[2] === archiveName ? [match[1]] : [];
  });
  if (matches.length !== 1) {
    throw new Error(`checksum list must contain exactly one entry for ${archiveName}`);
  }
  return matches[0];
}

async function fetchBytes(url) {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) throw new Error(`cannot download ${url}: HTTP ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}
