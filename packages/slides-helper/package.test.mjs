import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { readTarMembers } from "../../tools/verify-textlint-plugin-package.mjs";

const directory = fileURLToPath(new URL("./", import.meta.url));
const normalize = (text) => text.replace(/\r\n/g, "\n").replace(/[ \t]+$/gm, "");

test("npm pack includes the executable, exact dependencies, and complete distribution notices", async () => {
  const temporary = await mkdtemp(join(tmpdir(), "adocweave-slides-pack-"));
  try {
    const output = execFileSync("npm", ["pack", "--ignore-scripts", "--json", "--pack-destination", temporary], {
      cwd: directory, encoding: "utf8",
    });
    const [packed] = JSON.parse(output);
    const members = readTarMembers(await readFile(join(temporary, packed.filename)));
    const files = new Map(members.map(({ name, data }) => [name.slice("package/".length), data]));
    const manifest = JSON.parse(files.get("package.json").toString("utf8"));
    assert.equal(manifest.name, "@adocweave/slides-helper");
    assert.equal(manifest.version, "0.1.0");
    assert.equal(manifest.type, "module");
    assert.deepEqual(manifest.bin, { "adocweave-slides-helper": "./bin.mjs" });
    assert.equal(packed.files.find(({ path }) => path === "bin.mjs").mode & 0o111, 0o111);
    assert.match(files.get("bin.mjs").toString("utf8"), /^#!\/usr\/bin\/env node\n/);
    assert.deepEqual(manifest.dependencies, { "@mathjax/src": "4.1.3", citeproc: "2.4.63", parse5: "8.0.1" });
    assert.equal(manifest.devDependencies, undefined);
    assert.equal(manifest.bundledDependencies, undefined);
    assert.ok(![...files.keys()].some((path) => /(^|\/)(node_modules|fixtures)(\/|$)|\.test\.mjs$/.test(path)));
    const licenses = (await readdir(join(directory, "licenses"))).sort();
    const expected = manifest.files.filter((path) => path !== "licenses/")
      .concat("package.json", licenses.map((path) => `licenses/${path}`)).sort();
    assert.deepEqual([...files.keys()].sort(), expected);
    for (const path of ["THIRD_PARTY_NOTICES.adoc", "LICENSE-MIT", "LICENSE-APACHE", ...licenses.map((name) => `licenses/${name}`)]) {
      assert.equal(files.get(path).toString("utf8"), await readFile(join(directory, path), "utf8"));
    }
    assert.match(files.get("licenses/citeproc-CPAL.txt").toString("utf8"), /Exhibit B/);
    assert.match(files.get("licenses/mathjax-newcm-font-notices.txt").toString("utf8"), /GUST Font License/);
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
