import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { assetAuditInputs, validateSlidesRuntimeDependencies, validateArchive } from "./verify-slides-dependencies.mjs";

const read = (relative) => JSON.parse(readFileSync(new URL(relative, import.meta.url), "utf8"));
const manifest = read("../packages/slides-helper/package.json");
const lock = read("../packages/slides-helper/package-lock.json");
const evidence = read("../packages/slides-helper/licenses/evidence.json");
const sources = read("../crates/adocweave/assets/revealjs/sources.json");

test("helper archives are registry-only with complete integrity and exact direct versions", () => {
  assert.equal(validateSlidesRuntimeDependencies(manifest, lock, evidence).length, Object.keys(lock.packages).length - 1);
  for (const alter of [
    (value) => { value.packages["node_modules/parse5"].resolved = "https://registry.npmjs.org.evil.test/parse5.tgz"; },
    (value) => { value.packages["node_modules/parse5"].integrity = "sha512-AA=="; },
    (value) => { value.packages["node_modules/parse5"].license = "GPL-3.0"; },
    (value) => { value.packages["node_modules/parse5"].dev = true; },
    (value) => { value.packages["node_modules/parse5"].version = "8.0.2"; },
  ]) {
    const changed = structuredClone(lock);
    alter(changed);
    assert.throws(() => validateSlidesRuntimeDependencies(manifest, changed, evidence));
  }
});

test("CPAL and font decisions apply only to the reviewed archives and evidence files", () => {
  for (const name of ["citeproc", "@mathjax/mathjax-newcm-font"]) {
    const changed = structuredClone(lock);
    changed.packages[`node_modules/${name}`].integrity = `sha512-${"A".repeat(86)}==`;
    assert.throws(() => validateSlidesRuntimeDependencies(manifest, changed, evidence), /new archive license review/);
  }
  const broad = structuredClone(evidence);
  broad.parse5 = broad.citeproc;
  assert.throws(() => validateSlidesRuntimeDependencies(manifest, lock, broad), /two reviewed dependencies/);
  const missing = structuredClone(evidence);
  delete missing.citeproc.files["citeproc-CPAL.txt"];
  assert.throws(() => validateSlidesRuntimeDependencies(manifest, lock, missing));
  const wrong = structuredClone(evidence);
  wrong.citeproc.selectedLicense = "AGPL-3.0";
  assert.throws(() => validateSlidesRuntimeDependencies(manifest, lock, wrong));
});

test("every bundled browser archive is an npm audit input even when upstream uses devDependencies", () => {
  const { manifest, lock } = assetAuditInputs(sources);
  assert.deepEqual(Object.keys(manifest.dependencies), sources.archives.map(({ name }) => name));
  for (const archive of sources.archives) {
    const entry = lock.packages[`node_modules/${archive.name}`];
    assert.equal(entry.version, archive.version);
    assert.equal(entry.integrity, archive.integrity);
    assert.notEqual(entry.dev, true);
  }
  const duplicate = structuredClone(sources);
  duplicate.archives.push(duplicate.archives[0]);
  assert.throws(() => assetAuditInputs(duplicate), /Duplicate browser dependency/);
  assert.throws(() => validateArchive("fitty", "^2.4.2", sources.archives[1].tarball, sources.archives[1].integrity));
});
