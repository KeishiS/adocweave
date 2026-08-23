import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";

import {
  loadDistributionPlan,
  productAssetContracts,
  productVersion,
  selectProduct,
} from "./product-release.mjs";

const compareText = (left, right) => left < right ? -1 : left > right ? 1 : 0;

function fail(message) {
  throw new Error(message);
}

function canonicalJson(value) {
  const normalize = (entry) => {
    if (Array.isArray(entry)) return entry.map(normalize);
    if (entry && typeof entry === "object") {
      return Object.fromEntries(Object.keys(entry).sort().map((key) => [key, normalize(entry[key])]));
    }
    return entry;
  };
  return `${JSON.stringify(normalize(value), null, 2)}\n`;
}

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

export function buildMetadata(directory, sourceCommit, product, plan = loadDistributionPlan()) {
  if (!/^[0-9a-f]{40}$/.test(sourceCommit)) fail("source commit must be a lowercase 40-character Git commit");
  const selectedProduct = selectProduct(plan, product);
  const version = productVersion(selectedProduct);
  const archives = productAssetContracts(selectedProduct, plan, version).map((planned) => {
    let bytes;
    try {
      bytes = readFileSync(join(directory, planned.name));
    } catch {
      fail(`missing release archive: ${planned.name}`);
    }
    if (bytes.length === 0) fail(`empty release archive: ${planned.name}`);
    return { name: planned.name, sha256: sha256(bytes) };
  }).sort((left, right) => compareText(left.name, right.name));

  const manifestText = canonicalJson({
    assets: archives.map(({ name }) => ({ name })),
    product,
    productVersion: version,
    schemaVersion: 5,
    sourceCommit,
  });
  const checksums = [
    ...archives.map((archive) => [archive.name, archive.sha256]),
    ["adocweave-dist-manifest.json", sha256(manifestText)],
  ].sort(([left], [right]) => compareText(left, right));
  const checksumText = `${checksums.map(([name, digest]) => `${digest}  ${name}`).join("\n")}\n`;
  return { manifestText, checksumText };
}

export function writeMetadata(directory, sourceCommit, product, plan = loadDistributionPlan()) {
  const metadata = buildMetadata(directory, sourceCommit, product, plan);
  writeFileSync(join(directory, "adocweave-dist-manifest.json"), metadata.manifestText);
  writeFileSync(join(directory, "sha256.sum"), metadata.checksumText);
}

export function verifyMetadata(directory, sourceCommit, product, plan = loadDistributionPlan()) {
  const expected = buildMetadata(directory, sourceCommit, product, plan);
  for (const [name, text] of [
    ["adocweave-dist-manifest.json", expected.manifestText],
    ["sha256.sum", expected.checksumText],
  ]) {
    if (readFileSync(join(directory, name), "utf8") !== text) fail(`release metadata mismatch: ${name}`);
  }
  const entries = readdirSync(directory, { withFileTypes: true });
  if (entries.some((entry) => !entry.isFile())) {
    fail("release directory must contain public asset files only");
  }
  const actual = new Set(entries.map((entry) => entry.name));
  const selectedProduct = selectProduct(plan, product);
  const expectedNames = new Set([
    ...productAssetContracts(selectedProduct, plan, productVersion(selectedProduct))
      .map((asset) => asset.name),
    ...plan.releaseMetadata.map((entry) => entry.name),
  ]);
  if (actual.size !== expectedNames.size || [...actual].some((name) => !expectedNames.has(name))) {
    fail("release directory contains a missing, duplicate, or unplanned public asset");
  }
}

function main(args) {
  const [command, product, directoryArg, commitArg] = args;
  if (!new Set(["generate", "verify"]).has(command) || !product || !directoryArg) {
    fail("usage: release-metadata.mjs generate|verify PRODUCT ARTIFACT_DIRECTORY [SOURCE_COMMIT]");
  }
  const directory = resolve(directoryArg);
  const commit = commitArg ?? execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  if (command === "generate") writeMetadata(directory, commit, product);
  else verifyMetadata(directory, commit, product);
  process.stdout.write(`release metadata ${command}d: ${product} ${basename(directory)} @ ${commit}\n`);
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
