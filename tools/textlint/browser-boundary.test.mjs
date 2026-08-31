import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repositoryRoot = fileURLToPath(new URL("../../", import.meta.url));
const require = createRequire(import.meta.url);
function errorPayload(operation) {
  try {
    operation();
  } catch (error) {
    return JSON.parse(typeof error === "string" ? error : error.message);
  }
  assert.fail("WebAssembly呼出しが成功しました");
}

test("Browser用とtextlint用の公開項目を確認する", async () => {
  const browser = await import(
    new URL("../../target/adocweave-wasm-dev/adocweave_wasm.js", import.meta.url)
  );
  assert.deepEqual(
    Object.keys(browser).sort(),
    ["analyze", "default", "initSync", "protocolSchemaVersion"]
  );
  assert.deepEqual(
    Object.keys(
      require(
        `${repositoryRoot}target/adocweave-textlint-wasm-node/adocweave_textlint_wasm.js`
      )
    ),
    ["parseText"]
  );
});

test("実WebAssembly境界がrequest上限をcode付きで拒否する", () => {
  const { parseText } = require(
    `${repositoryRoot}target/adocweave-textlint-wasm-node/adocweave_textlint_wasm.js`
  );
  assert.equal(
    errorPayload(() => parseText({
      source: "x".repeat(10 * 1024 * 1024 + 1),
      sourceId: null
    })).code,
    "input-too-large"
  );
  assert.equal(
    errorPayload(() => parseText({
      source: "",
      sourceId: "x".repeat(4 * 1024 + 1)
    })).code,
    "invalid-request"
  );
});
