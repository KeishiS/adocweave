import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { readTarMembers } from "../../tools/verify-textlint-plugin-package.mjs";
import { verifyInstalledHelper } from "../../tools/slides-helper-release-smoke.mjs";

const directory = fileURLToPath(new URL("./", import.meta.url));
const normalize = (text) => text.replace(/\r\n/g, "\n").replace(/[ \t]+$/gm, "");

test("npm pack includes the executable, exact dependencies, and complete distribution notices", async () => {
  const temporary = await mkdtemp(join(tmpdir(), "adocweave-slides-pack-"));
  try {
    const output = execFileSync("npm", ["pack", "--ignore-scripts", "--json", "--pack-destination", temporary], {
      cwd: directory, encoding: "utf8",
    });
    const [packed] = JSON.parse(output);
    const members = readTarMembers(await readFile(join(temporary, packed.filename)), { maximumTarBytes: 256 * 1024 * 1024 });
    const files = new Map(members.map(({ name, data }) => [name.slice("package/".length), data]));
    const manifest = JSON.parse(files.get("package.json").toString("utf8"));
    const sourceManifest = JSON.parse(await readFile(join(directory, "package.json"), "utf8"));
    assert.equal(manifest.name, "@adocweave/slides-helper");
    assert.equal(manifest.version, sourceManifest.version);
    assert.equal(manifest.type, "module");
    assert.deepEqual(manifest.bin, { "adocweave-slides-helper": "./bin.mjs" });
    assert.equal(packed.files.find(({ path }) => path === "bin.mjs").mode & 0o111, 0o111);
    assert.match(files.get("bin.mjs").toString("utf8"), /^#!\/usr\/bin\/env node\n/);
    assert.deepEqual(manifest.dependencies, sourceManifest.dependencies);
    assert.equal(manifest.devDependencies, undefined);
    assert.equal(manifest.bundleDependencies, true);
    assert.ok(![...files.keys()].filter(path => !path.startsWith("node_modules/")).some((path) => /(^|\/)fixtures(\/|$)|\.test\.mjs$/.test(path)));
    const licenses = (await readdir(join(directory, "licenses"))).sort();
    const expected = manifest.files.filter((path) => path !== "licenses/")
      .concat("package.json", licenses.map((path) => `licenses/${path}`)).sort();
    assert.deepEqual([...files.keys()].filter(path => !path.startsWith("node_modules/")).sort(), expected);
    for (const path of ["THIRD_PARTY_NOTICES.adoc", "LICENSE-MIT", "LICENSE-APACHE", ...licenses.map((name) => `licenses/${name}`)]) {
      assert.equal(files.get(path).toString("utf8"), await readFile(join(directory, path), "utf8"));
    }
    assert.match(files.get("licenses/citeproc-CPAL.txt").toString("utf8"), /Exhibit B/);
    assert.match(files.get("licenses/mathjax-newcm-font-notices.txt").toString("utf8"), /GUST Font License/);
    const lock = JSON.parse(await readFile(join(directory, "package-lock.json"), "utf8"));
    const bundled = [...files].filter(([path]) => /^(?:node_modules\/(?:@[^/]+\/)?[^/]+\/)+package\.json$/.test(path));
    assert.deepEqual(new Map(bundled.map(([path, bytes]) => [path.slice(0, -13), JSON.parse(bytes).version])),
      new Map(Object.entries(lock.packages).filter(([path]) => path).map(([path, item]) => [path, item.version])));
    await verifyInstalledHelper(join(temporary, packed.filename));
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

test("recorded dependency notices match installed archive texts rather than metadata alone", async () => {
  const lock = JSON.parse(await readFile(join(directory, "package-lock.json"), "utf8"));
  for (const path of Object.keys(lock.packages).filter((path) => path.startsWith("node_modules/"))) {
    const name = path.slice("node_modules/".length);
    if (name === "@mathjax/mathjax-newcm-font") continue;
    const entries = await readdir(join(directory, path));
    const license = entries.find((entry) => /^licen[sc]e(?:\.[^.]+)?$/i.test(entry));
    assert.ok(license, `${name} must have a distributed license text`);
    const recorded = await readFile(join(directory, "licenses", `${name.replaceAll("/", "-")}.txt`), "utf8");
    assert.equal(recorded, normalize(await readFile(join(directory, path, license), "utf8")), name);
  }
});
