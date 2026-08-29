import assert from "node:assert/strict";
import test from "node:test";

import { analysisPayload, parseWasmError } from "./analysis.mjs";
import { createDirectAnalyzer, defaultDirectAssetUrls } from "./direct.mjs";
import { PROTOCOL_SCHEMA_VERSION } from "./worker-protocol.mjs";

const ASSETS = { moduleUrl: "https://example.test/wasm.js", wasmUrl: "https://example.test/wasm.wasm" };

test("同梱WebAssemblyの位置を入口からの相対で求める", () => {
  const { moduleUrl, wasmUrl } = defaultDirectAssetUrls("file:///pkg/worker/direct.mjs");
  assert.equal(moduleUrl.href, "file:///pkg/wasm/adocweave_wasm.js");
  assert.equal(wasmUrl.href, "file:///pkg/wasm/adocweave_wasm_bg.wasm");
});

test("要求をWorkerの入口と同じ形のまま渡す", () => {
  const request = { source: { text: "= T\n" }, products: { html: true } };
  assert.equal(analysisPayload(request), request);
});

test("cloneできない要求をinvalid-requestとして拒否する", () => {
  assert.throws(
    () => analysisPayload({ source: { text: "Text" }, products: { html: true }, value: () => {} }),
    (error) => error.code === "invalid-request",
  );
});

test("WASMの構造化errorだけをcode付きで読む", () => {
  assert.deepEqual(parseWasmError({ code: "input-limit-exceeded", message: "too large" }), {
    code: "input-limit-exceeded",
    message: "too large"
  });
  assert.equal(parseWasmError("boom"), null);
  assert.equal(parseWasmError(new Error("boom")), null);
  assert.equal(parseWasmError({ code: "unknown", message: "boom" }), null);
});

test("protocol schemaが違うWebAssemblyを拒否する", async () => {
  await assert.rejects(
    createDirectAnalyzer({
      ...ASSETS,
      import: async () => ({ initSync() {}, protocolSchemaVersion: () => PROTOCOL_SCHEMA_VERSION + 1 }),
      readWasm: () => new Uint8Array()
    }),
    /expected WASM protocol schema/u
  );
});

test("analyze exportを持たないWebAssemblyを拒否する", async () => {
  await assert.rejects(
    createDirectAnalyzer({
      ...ASSETS,
      import: async () => ({ initSync() {}, protocolSchemaVersion: () => PROTOCOL_SCHEMA_VERSION }),
      readWasm: () => new Uint8Array()
    }),
    /analyze export is missing/u
  );
});

test("構造化errorはcodeを保ち、trapはwasm-trappedになる", async () => {
  const analyzer = await createDirectAnalyzer({
    ...ASSETS,
    import: async () => ({
      initSync() {},
      protocolSchemaVersion: () => PROTOCOL_SCHEMA_VERSION,
      analyze: (payload) => {
        if (payload === null || typeof payload !== "object") {
          throw { code: "invalid-request", message: "invalid request" };
        }
        if (payload.source.text === "structured") {
          throw { code: "invalid-request", message: "bad" };
        }
        throw new Error("trapped");
      }
    }),
    readWasm: () => new Uint8Array()
  });
  assert.throws(() => analyzer.analyze({
    source: { text: "structured" }, products: { html: true },
  }), (error) => {
    assert.equal(error.code, "invalid-request");
    return true;
  });
  for (const request of [null, 1, "text"]) {
    assert.throws(() => analyzer.analyze(request), (error) => {
      assert.equal(error.code, "invalid-request");
      return true;
    });
  }
  assert.throws(() => analyzer.analyze({
    source: { text: "other" }, products: { html: true },
  }), (error) => {
    assert.equal(error.code, "wasm-trapped");
    return true;
  });
});
