import assert from "node:assert/strict";
import { appendFileSync, cpSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { verifySlidesAssets } from "./verify-slides-assets.mjs";

test("pinned browser runtime and speaker inline hashes verify without npm or network", () => {
  assert.equal(verifySlidesAssets().speakerScripts.length, 1);
});

test("changing executable asset bytes refuses the bundle before executing the notes plugin", () => {
  const directory = mkdtempSync(join(tmpdir(), "adocweave-assets-"));
  try {
    cpSync(fileURLToPath(new URL("../crates/adocweave/assets/revealjs/", import.meta.url)), directory, { recursive: true });
    appendFileSync(join(directory, "notes.js"), "\nthrow new Error('changed asset');\n");
    assert.throws(() => verifySlidesAssets(directory), /asset digest: notes.js/);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
