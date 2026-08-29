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

test("要求を一回の走査で固定したsnapshotとして渡す", () => {
  const request = { source: { text: "= T\n" }, products: { html: true } };
  const payload = analysisPayload(request);
  assert.notEqual(payload, request);
  assert.notEqual(payload.source, request.source);
  assert.deepEqual(payload, request);
});

test("後方のProxy trapが検査済みdata propertyを変えても元要求を再読しない", () => {
  const source = { text: "Text" };
  let getterCalls = 0;
  const request = new Proxy({ source, products: { symbols: true } }, {
    getOwnPropertyDescriptor(target, key) {
      if (key === "products") {
        Object.defineProperty(source, "text", {
          configurable: true,
          enumerable: true,
          get() {
            getterCalls += 1;
            return "changed";
          },
        });
      }
      return Reflect.getOwnPropertyDescriptor(target, key);
    },
  });
  const payload = analysisPayload(request);
  assert.equal(payload.source.text, "Text");
  assert.equal(getterCalls, 0);
});

test("JS snapshotはWASMと同じ固定上限でdescriptor走査を停止する", () => {
  const tooManyKeys = {};
  for (let index = 0; index < 20_001; index += 1) tooManyKeys[`key${index}`] = true;
  let descriptorCalls = 0;
  const observed = new Proxy(tooManyKeys, {
    getOwnPropertyDescriptor(target, key) {
      descriptorCalls += 1;
      return Reflect.getOwnPropertyDescriptor(target, key);
    },
  });
  assert.throws(
    () => analysisPayload(observed),
    (error) => error.code === "input-limit-exceeded" && /object key count/u.test(error.message),
  );
  assert.equal(descriptorCalls, 0);

  const tooManyNodes = {};
  for (let branch = 0; branch < 5; branch += 1) {
    const values = {};
    for (let index = 0; index < 19_999; index += 1) values[`key${index}`] = true;
    tooManyNodes[`branch${branch}`] = values;
  }
  assert.throws(
    () => analysisPayload(tooManyNodes),
    (error) => error.code === "input-limit-exceeded" && /node count/u.test(error.message),
  );

  let deep = "Text";
  for (let depth = 0; depth < 128; depth += 1) deep = { next: deep };
  for (const request of [deep, new Array(20_001), { ["x".repeat(1_025)]: true }]) {
    assert.throws(
      () => analysisPayload(request),
      (error) => error.code === "input-limit-exceeded",
    );
  }

  assert.throws(
    () => analysisPayload({ value: "x".repeat(16 * 1024 * 1024 + 1) }),
    (error) => error.code === "input-limit-exceeded" && /string length/u.test(error.message),
  );
  assert.throws(
    () => analysisPayload({ value: "😀".repeat(8 * 1024 * 1024) }),
    (error) => error.code === "input-limit-exceeded" && /string bytes/u.test(error.message),
  );
});

test("cloneできない要求をinvalid-requestとして拒否する", () => {
  assert.throws(
    () => analysisPayload({ source: { text: "Text" }, products: { html: true }, value: () => {} }),
    (error) => error.code === "invalid-request",
  );
});

test("structured cloneでplain objectへ変わるinstanceを共通境界で拒否する", () => {
  class CustomRequest {
    constructor() {
      this.source = { text: "Text" };
      this.products = { html: true };
    }
  }
  assert.throws(
    () => analysisPayload(new CustomRequest()),
    (error) => error.code === "invalid-request",
  );
});

test("accessorを実行せずWorkerとdirectの共通境界で拒否する", () => {
  let getterCalls = 0;
  const request = { source: { text: "Text" }, products: { html: true } };
  Object.defineProperty(request, "resources", {
    enumerable: true,
    get() {
      getterCalls += 1;
      return {};
    },
  });
  assert.throws(
    () => analysisPayload(request),
    (error) => error.code === "invalid-request",
  );
  assert.equal(getterCalls, 0);
});

test("direct入口はaccessorをWASMへ渡さず共通errorを返す", async () => {
  let analyzeCalls = 0;
  const analyzer = await createDirectAnalyzer({
    ...ASSETS,
    import: async () => ({
      initSync() {},
      protocolSchemaVersion: () => PROTOCOL_SCHEMA_VERSION,
      analyze: () => {
        analyzeCalls += 1;
        return {};
      },
    }),
    readWasm: () => new Uint8Array(),
  });
  let getterCalls = 0;
  const request = { source: { text: "Text" }, products: { html: true } };
  Object.defineProperty(request, "resources", {
    enumerable: true,
    get() {
      getterCalls += 1;
      return {};
    },
  });
  assert.throws(() => analyzer.analyze(request), (error) => {
    assert.equal(error.code, "invalid-request");
    assert.equal(error.message, "the analysis request must be structured-cloneable");
    return true;
  });
  assert.equal(getterCalls, 0);
  assert.equal(analyzeCalls, 0);
});

test("direct入口は固定上限をWASM呼出し前に共通codeで返す", async () => {
  let analyzeCalls = 0;
  const analyzer = await createDirectAnalyzer({
    ...ASSETS,
    import: async () => ({
      initSync() {},
      protocolSchemaVersion: () => PROTOCOL_SCHEMA_VERSION,
      analyze: () => {
        analyzeCalls += 1;
        return {};
      },
    }),
    readWasm: () => new Uint8Array(),
  });
  const request = { source: { text: "x".repeat(16 * 1024 * 1024 + 1) }, products: { html: true } };
  assert.throws(() => analyzer.analyze(request), (error) => {
    assert.equal(error.code, "input-limit-exceeded");
    return true;
  });
  assert.equal(analyzeCalls, 0);
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
