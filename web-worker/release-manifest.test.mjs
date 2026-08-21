import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { BROWSER_PACKAGE_VERSION, PACKAGE_VERSION } from "./contracts.mjs";

test("worker consumes the public WASM contract registry", async () => {
  const manifestUrl = new URL("../release-manifest.json", import.meta.url);
  const manifest = JSON.parse(await readFile(manifestUrl, "utf8"));

  // 3製品共通のrelease manifest schema(version 1)と、AdocWeave固有の拡張項目distributionPlan。
  assert.deepEqual(Object.keys(manifest).sort(), [
    "assets",
    "distributionPlan",
    "nodeVersion",
    "packageVersion",
    "product",
    "releaseNotes",
    "rustVersion",
    "schemaVersion",
  ]);
  assert.equal(manifest.schemaVersion, 1);
  assert.equal(manifest.product, "adocweave");
  assert.equal(manifest.releaseNotes, "release/notes.md");
  assert.deepEqual(manifest.assets, []);
  assert.equal(manifest.distributionPlan, "release/distribution-plan.json");
  assert.equal(manifest.packageVersion, PACKAGE_VERSION);
  assert.equal(manifest.packageVersion, BROWSER_PACKAGE_VERSION);
  assert.match(manifest.rustVersion, /^\d+\.\d+\.\d+$/);
  assert.match(manifest.nodeVersion, /^\d+\.\d+\.\d+$/);
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
