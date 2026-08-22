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

test("READMEはPromise単一入口と配備条件を説明する", async () => {
  const readme = await readFile(new URL("./README.adoc", import.meta.url), "utf8");

  assert.match(readme, /analyze\(request, \{ signal \}\).*Promise/);
  assert.match(readme, /一つのclientは同時に一つの解析/);
  assert.match(readme, /AbortController/);
  assert.match(readme, /取消し.*Workerを終了し.*Promiseをreject/s);
  assert.match(readme, /WASM protocolの.*schema handshake/s);
  assert.match(readme, /defaultAssetUrls\(baseUrl\).*配備後の公開entry/s);
  assert.match(readme, /同一origin/);
  assert.match(readme, /script-src.*worker-src.*connect-src/s);
  assert.doesNotMatch(readme, /onResult|onError|generation|SharedArrayBuffer|COOP|COEP|packageVersion/);
});
