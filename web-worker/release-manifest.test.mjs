import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { BROWSER_PACKAGE_VERSION } from "./contracts.mjs";

test("Browser packageの公開versionはpackage.jsonを正本とする", async () => {
  const manifestUrl = new URL("./package.json", import.meta.url);
  const manifest = JSON.parse(await readFile(manifestUrl, "utf8"));

  assert.equal(manifest.name, "@adocweave/browser");
  assert.equal(manifest.version, BROWSER_PACKAGE_VERSION);
});
