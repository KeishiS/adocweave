import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("WebAssembly packageの識別子とversionはpackage.jsonを正本とする", async () => {
  const manifestUrl = new URL("./package.json", import.meta.url);
  const manifest = JSON.parse(await readFile(manifestUrl, "utf8"));

  assert.equal(manifest.name, "@adocweave/wasm");
  assert.match(manifest.version, /^\d+\.\d+\.\d+$/u);
  assert.equal(manifest.repository.directory, "packages/wasm");
});
