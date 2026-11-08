import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const directory = fileURLToPath(new URL("./", import.meta.url));
const normalize = (text) => text.replace(/\r\n/g, "\n").replace(/[ \t]+$/gm, "");

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
