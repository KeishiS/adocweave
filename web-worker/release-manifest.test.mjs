import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { WASM_PACKAGE_VERSION } from "./contracts.mjs";

test("WebAssembly packageの公開versionはpackage.jsonを正本とする", async () => {
  const manifestUrl = new URL("./package.json", import.meta.url);
  const manifest = JSON.parse(await readFile(manifestUrl, "utf8"));

  assert.equal(manifest.name, "@adocweave/wasm");
  assert.equal(manifest.version, WASM_PACKAGE_VERSION);
});
