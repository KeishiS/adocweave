import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { verifySlidesAssets } from "./verify-slides-assets.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const helper = join(root, "packages/slides-helper");
const allowedLicenses = new Set(["Apache-2.0", "BlueOak-1.0.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "MIT"]);
const exceptions = new Map([
  ["citeproc", { license: "CPAL-1.0", files: ["citeproc.txt", "citeproc-CPAL.txt", "citeproc-attribution.txt"] }],
  ["@mathjax/mathjax-newcm-font", {
    license: "GUST-Font-License / LPPL-1.3c-or-later",
    files: ["mathjax-newcm-font-notices.txt", "GUST-FONT-LICENSE.txt", "LPPL-1.3c.txt"],
  }],
]);
const json = (path) => JSON.parse(readFileSync(path, "utf8"));
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const normalize = (text) => text.replace(/\r\n/g, "\n").replace(/[ \t]+$/gm, "");

export function validateArchive(name, version, resolved, integrity) {
  assert.match(name, /^(?:@[a-z0-9._-]+\/)?[a-z0-9._-]+$/i, "Invalid npm package name.");
  assert.match(version, /^\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?$/, "Expected an exact package version.");
  const basename = name.slice(name.lastIndexOf("/") + 1);
  assert.equal(resolved, `https://registry.npmjs.org/${name}/-/${basename}-${version}.tgz`, "Expected the fixed npm registry archive.");
  assert.match(integrity, /^sha512-[A-Za-z0-9+/]{86}==$/, "Expected a SHA-512 archive integrity.");
}

export function validateSlidesRuntimeDependencies(manifest, lock, evidence) {
  assert.equal(manifest.name, "@adocweave/slides-helper");
  assert.equal(lock.lockfileVersion, 3);
  assert.equal(lock.packages?.[""]?.name, manifest.name);
  assert.equal(lock.packages[""].version, manifest.version);
  assert.deepEqual(lock.packages[""].dependencies, manifest.dependencies);
  assert.deepEqual(Object.keys(evidence).sort(), [...exceptions.keys()].sort(), "License evidence is limited to the two reviewed dependencies.");
  const packages = [];
  for (const [path, entry] of Object.entries(lock.packages)) {
    if (!path) continue;
    assert.ok(path.startsWith("node_modules/") && !path.split("/").some((part) => ["", ".", ".."].includes(part)), "Invalid dependency path.");
    assert.notEqual(entry.dev, true, "The helper has no development dependency tree.");
    const name = path.slice(path.lastIndexOf("node_modules/") + "node_modules/".length);
    validateArchive(name, entry.version, entry.resolved, entry.integrity);
    if (exceptions.has(name)) {
      const recorded = evidence[name];
      assert.equal(recorded.version, entry.version, `${name} requires a new archive license review.`);
      assert.equal(recorded.integrity, entry.integrity, `${name} requires a new archive license review.`);
      assert.equal(recorded.metadataLicense, entry.license);
      assert.equal(recorded.selectedLicense, exceptions.get(name).license);
      assert.deepEqual(Object.keys(recorded.files).sort(), exceptions.get(name).files.toSorted());
    } else {
      assert.ok(allowedLicenses.has(entry.license), `Unapproved helper runtime license: ${name} ${entry.license}.`);
    }
    packages.push({ path, name, ...entry });
  }
  for (const [name, version] of Object.entries(manifest.dependencies)) {
    assert.equal(lock.packages[`node_modules/${name}`]?.version, version, "Direct dependencies must be pinned to exact installed versions.");
  }
  for (const name of exceptions.keys()) assert.ok(packages.some((pkg) => pkg.name === name), `Missing license-reviewed dependency: ${name}.`);
  return packages.toSorted((left, right) => left.name.localeCompare(right.name));
}

export function verifyInstalledNotices(directory, packages, evidence) {
  for (const pkg of packages) {
    const installed = json(join(directory, pkg.path, "package.json"));
    assert.equal(installed.version, pkg.version, `Installed version: ${pkg.name}.`);
    assert.equal(installed.license, pkg.license, `Installed metadata: ${pkg.name}.`);
    if (pkg.name === "@mathjax/mathjax-newcm-font") continue;
    const license = readdirSync(join(directory, pkg.path)).find((name) => /^licen[sc]e(?:\.[^.]+)?$/i.test(name));
    assert.ok(license, `Missing distributed license: ${pkg.name}.`);
    const recorded = readFileSync(join(directory, "licenses", `${pkg.name.replaceAll("/", "-")}.txt`), "utf8");
    assert.equal(recorded, normalize(readFileSync(join(directory, pkg.path, license), "utf8")), `Distributed license changed: ${pkg.name}.`);
  }
  for (const entry of Object.values(evidence)) {
    for (const [path, sha256] of Object.entries(entry.files)) {
      assert.equal(digest(readFileSync(join(directory, "licenses", path))), sha256, `License evidence changed: ${path}.`);
    }
  }
}

export function assetAuditInputs(sources) {
  assert.equal(sources.schemaVersion, 1);
  const dependencies = {};
  const packages = {};
  for (const archive of sources.archives) {
    validateArchive(archive.name, archive.version, archive.tarball, archive.integrity);
    assert.ok(!Object.hasOwn(dependencies, archive.name), "Duplicate browser dependency.");
    dependencies[archive.name] = archive.version;
    packages[`node_modules/${archive.name}`] = {
      version: archive.version, resolved: archive.tarball, integrity: archive.integrity,
    };
  }
  const manifest = { name: "adocweave-slides-assets-audit", version: "0.0.0", private: true, dependencies };
  const lock = { name: manifest.name, version: manifest.version, lockfileVersion: 3, packages: { "": manifest, ...packages } };
  return { manifest, lock };
}

function audit(directory) {
  const result = spawnSync("npm", ["audit", "--omit=dev", "--prefix", directory], { stdio: "inherit" });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, "npm runtime audit failed.");
}

export function main(args = process.argv.slice(2)) {
  assert.ok(args.length === 0 || (args.length === 1 && args[0] === "--audit"), "Usage: node tools/verify-slides-dependencies.mjs [--audit]");
  const evidence = json(join(helper, "licenses/evidence.json"));
  const packages = validateSlidesRuntimeDependencies(json(join(helper, "package.json")), json(join(helper, "package-lock.json")), evidence);
  verifyInstalledNotices(helper, packages, evidence);
  const assets = verifySlidesAssets();
  const inputs = assetAuditInputs(assets);
  if (args.includes("--audit")) {
    audit(helper);
    const temporary = mkdtempSync(join(tmpdir(), "adocweave-slides-assets-audit-"));
    try {
      writeFileSync(join(temporary, "package.json"), JSON.stringify(inputs.manifest));
      writeFileSync(join(temporary, "package-lock.json"), JSON.stringify(inputs.lock));
      audit(temporary);
    } finally {
      rmSync(temporary, { recursive: true, force: true });
    }
  }
  process.stdout.write(`Slides runtime sources and license notices verified: ${packages.length} helper packages, ${assets.archives.length} bundled browser packages.\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try { main(); } catch (error) { process.stderr.write(`${error.message}\n`); process.exitCode = 1; }
}
