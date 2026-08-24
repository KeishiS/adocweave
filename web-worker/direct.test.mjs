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

test("要求の既定値をWorkerの入口と同じ形へ揃える", () => {
  const payload = analysisPayload({ source: "= T\n" });
  assert.deepEqual(payload, {
    sourceId: null,
    source: "= T\n",
    analysisOptions: {},
    renderPolicy: {},
    outputLimits: {}
  });
  assert.equal(Object.hasOwn(analysisPayload({ source: "" }), "products"), false);
  assert.deepEqual(analysisPayload({ source: "", products: { html: true } }).products, { html: true });
});

test("WASMの構造化errorだけをcode付きで読む", () => {
  assert.deepEqual(parseWasmError('{"code":"limit","message":"too large"}'), {
    code: "limit",
    message: "too large"
  });
  assert.equal(parseWasmError("boom"), null);
  assert.equal(parseWasmError(new Error("boom")), null);
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

test("process exportを持たないWebAssemblyを拒否する", async () => {
  await assert.rejects(
    createDirectAnalyzer({
      ...ASSETS,
      import: async () => ({ initSync() {}, protocolSchemaVersion: () => PROTOCOL_SCHEMA_VERSION }),
      readWasm: () => new Uint8Array()
    }),
    /process export is missing/u
  );
});

test("構造化errorはcodeを保ち、trapはwasm-trappedになる", async () => {
  const analyzer = await createDirectAnalyzer({
    ...ASSETS,
    import: async () => ({
      initSync() {},
      protocolSchemaVersion: () => PROTOCOL_SCHEMA_VERSION,
      process: (payload) => {
        if (payload.source === "structured") throw '{"code":"invalid-request","message":"bad"}';
        throw new Error("trapped");
      }
    }),
    readWasm: () => new Uint8Array()
  });
  assert.throws(() => analyzer.analyze({ source: "structured" }), (error) => {
    assert.equal(error.code, "invalid-request");
    return true;
  });
  assert.throws(() => analyzer.analyze({ source: "other" }), (error) => {
    assert.equal(error.code, "wasm-trapped");
    return true;
  });
});
