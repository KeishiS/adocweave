import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import process from "node:process";

import {
  expectedPublishedReleaseAssets,
  expectedReleaseAssets,
  validateDistPlan,
  validateReleaseTag,
} from "./native-release-checks.mjs";

function fail(message) {
  throw new Error(message);
}

function sameNames(actual, expected) {
  return JSON.stringify([...actual].sort()) === JSON.stringify([...expected].sort());
}

function checksums(source, label) {
  const entries = new Map();
  for (const line of source.trim().split(/\r?\n/u)) {
    const match = /^([0-9a-f]{64})[ \t]+\*?([^/\\]+)$/iu.exec(line.trim());
    if (!match || entries.has(match[2])) fail(`invalid checksum entry in ${label}`);
    entries.set(match[2], match[1].toLowerCase());
  }
  if (entries.size === 0) fail(`empty checksum file: ${label}`);
  return entries;
}

function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

export function validatePublishedNativeRelease({ directory, release, tag }) {
  validateReleaseTag(tag);
  if (release.tag_name !== tag || release.draft !== false || release.prerelease !== false) {
    fail("GitHub Release must be the requested stable tag");
  }

  const expected = expectedPublishedReleaseAssets();
  const remoteAssets = (release.assets ?? []).map(({ name, size }) => {
    if (!Number.isSafeInteger(size) || size <= 0) fail(`empty GitHub Release asset: ${name}`);
    return name;
  });
  if (!sameNames(remoteAssets, expected)) fail("GitHub Release asset set mismatch");

  const root = resolve(directory);
  const localAssets = readdirSync(root, { withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => entry.name);
  if (!sameNames(localAssets, expected)) fail("downloaded native Release asset set mismatch");

  const manifest = JSON.parse(readFileSync(join(root, "dist-manifest.json"), "utf8"));
  validateDistPlan(manifest, tag);

  const archives = expectedReleaseAssets().filter((name) => name.endsWith(".zip"));
  const actualDigests = new Map(
    archives.map((name) => [name, digest(readFileSync(join(root, name)))]),
  );
  for (const name of archives) {
    const checksumName = `${name}.sha256`;
    const entries = checksums(readFileSync(join(root, checksumName), "utf8"), checksumName);
    if (!sameNames(entries.keys(), [name]) || entries.get(name) !== actualDigests.get(name)) {
      fail(`individual checksum mismatch: ${name}`);
    }
  }

  const unified = checksums(readFileSync(join(root, "sha256.sum"), "utf8"), "sha256.sum");
  if (!sameNames(unified.keys(), archives)) fail("sha256.sum archive set mismatch");
  for (const name of archives) {
    if (unified.get(name) !== actualDigests.get(name)) fail(`sha256.sum mismatch: ${name}`);
  }

  return { assets: expected, manifest: basename(join(root, "dist-manifest.json")), tag };
}

export function main(args) {
  if (args.length !== 3) {
    fail("usage: node tools/native-release-acceptance.mjs TAG RELEASE_JSON ASSET_DIRECTORY");
  }
  const [tag, releasePath, directory] = args;
  const release = JSON.parse(readFileSync(resolve(releasePath), "utf8"));
  const result = validatePublishedNativeRelease({ directory, release, tag });
  process.stdout.write(`native Release assets verified: ${result.tag}\n`);
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
