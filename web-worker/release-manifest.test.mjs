import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { BROWSER_PACKAGE_VERSION, PACKAGE_VERSION } from "./contracts.mjs";

test("Browser packageの公開versionはpackage.jsonを正本とする", async () => {
  const manifestUrl = new URL("./package.json", import.meta.url);
  const manifest = JSON.parse(await readFile(manifestUrl, "utf8"));

  assert.equal(manifest.name, "@adocweave/browser");
  assert.equal(manifest.version, PACKAGE_VERSION);
  assert.equal(manifest.version, BROWSER_PACKAGE_VERSION);
});

test("READMEはBrowserのversion境界とprojection境界を説明する", async () => {
  const readme = await readFile(new URL("./README.adoc", import.meta.url), "utf8");

  assert.match(readme, /unsupported-package-version/);
  assert.match(readme, /Worker応答の.*version.*解析要求の.*version/s);
  assert.match(readme, /invalid-worker-response/);
  assert.match(readme, /staleな応答.*onResult.*onError.*通知しません/s);
  assert.match(readme, /onError.*microtask/s);
  assert.match(readme, /WASM adapterがcoreから受け取るprojection JSON/);
  assert.match(readme, /内部の取り決めにないfieldを検出した場合は処理を失敗/);
});
