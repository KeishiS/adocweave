import assert from "node:assert/strict";
import { appendFileSync, cpSync, mkdtempSync, rmSync, readFileSync } from "node:fs";
import vm from "node:vm";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { verifySlidesAssets } from "./verify-slides-assets.mjs";

test("pinned browser runtime and speaker inline hashes verify without npm or network", () => {
  assert.equal(verifySlidesAssets().speakerScripts.length, 1);
});

test("cross-slide references expose only the target's own fragment and preserve visible stages", async () => {
  const source = readFileSync(new URL("../crates/adocweave/assets/slides/bootstrap.js", import.meta.url), "utf8");
  let click;
  let current = { h: 0, v: 0, f: -1 };
  const calls = [];
  const slide = { indices: { h: 1, v: 0 } };
  const target = { fragment: null, closest(selector) { return selector === ".fragment" ? this.fragment : slide; }, querySelector() { throw new Error("a target's child fragment is unrelated"); } };
  const context = {
    Reveal: class { getIndices(value) { return value ? value.indices : current; } slide(h, v, f) { calls.push(["slide", h, v, f]); } navigateFragment(f) { calls.push(["fragment", f]); } initialize() { return Promise.resolve(); } layout() {} },
    document: { body: { dataset: { audience: "public" } }, querySelector() { return {}; }, addEventListener(event, callback, capture) { if (event === "click") { assert.equal(capture, true); click = callback; } }, getElementById() { return target; }, images: [] },
    window: { dispatchEvent() {} }, location: { hash: "#/" }, Event: class {}
  };
  vm.runInNewContext(source, context);
  const anchor = { closest() { return this; }, getAttribute() { return "#target"; } };
  const go = () => click({ target: anchor, preventDefault() {}, stopPropagation() {} });
  go();
  assert.deepEqual(calls.splice(0), [["slide", 1, 0, -1], ["fragment", -1]]);
  target.fragment = { dataset: { fragmentIndex: "2" } };
  go();
  assert.deepEqual(calls.splice(0), [["slide", 1, 0, 2], ["fragment", 2]]);
  current = { h: 1, v: 0, f: 3 };
  go();
  assert.deepEqual(calls.splice(0), [["slide", 1, 0, 3], ["fragment", 3]]);
  target.fragment = null;
  go();
  assert.deepEqual(calls.splice(0), [["slide", 1, 0, 3], ["fragment", 3]]);
  await Promise.resolve();
});

test("changing executable asset bytes refuses the bundle before executing the notes plugin", () => {
  const directory = mkdtempSync(join(tmpdir(), "adocweave-assets-"));
  try {
    cpSync(fileURLToPath(new URL("../crates/adocweave/assets/revealjs/", import.meta.url)), directory, { recursive: true });
    appendFileSync(join(directory, "notes.js"), "\nthrow new Error('changed asset');\n");
    assert.throws(() => verifySlidesAssets(directory), /asset digest: notes.js/);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
