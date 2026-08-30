import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { Worker } from "node:worker_threads";

import { PROTOCOL_SCHEMA_VERSION } from "./worker-protocol.mjs";
import { analysisPayload } from "./analysis.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(here, "../..");
const require = createRequire(import.meta.url);
const wasmModule = resolve(
  repositoryRoot,
  "target/adocweave-wasm-node/adocweave_wasm.js",
);
const wasm = require(wasmModule);

test("generated wasm-bindgen module comes from the repository target directory", () => {
  assert.equal(resolve(repositoryRoot, "packages/wasm"), here);
  assert.equal(
    wasmModule,
    resolve(repositoryRoot, "target/adocweave-wasm-node/adocweave_wasm.js"),
  );
});

function wasmError(operation) {
  try {
    operation();
  } catch (error) {
    assert.equal(typeof error, "object");
    assert.equal(typeof error.code, "string");
    assert.equal(typeof error.message, "string");
    return error;
  }
  assert.fail("WASM request unexpectedly succeeded");
}

test("generated wasm-bindgen accepts the public request and returns selected products", () => {
  assert.equal(wasm.protocolSchemaVersion(), PROTOCOL_SCHEMA_VERSION);
  assert.equal(typeof wasm.analyze, "function");
  assert.equal(Object.hasOwn(wasm, "process"), false);
  assert.equal(Object.hasOwn(wasm, "preprocess"), false);

  const response = wasm.analyze({
    source: { text: "= Title\n\nText" },
    products: { html: true, symbols: true, document: true },
  });
  assert.deepEqual(Object.keys(response).sort(), ["document", "html", "symbols"]);
  assert.match(response.html, /<h1[^>]*>Title<\/h1>/u);
  assert.equal(response.symbols[0].name, "Title");
  assert.equal(response.document.title.text, "Title");
});

test("generated wasm-bindgen rejects unknown fields and invalid product values", () => {
  for (const request of [
    { source: { text: "Text", unknown: true }, products: { html: true } },
    { source: { text: "Text" }, products: { html: true }, unknown: true },
    { source: { text: "Text" }, products: { hmtl: true } },
    { source: { text: "Text" }, products: { html: false } },
    { source: { text: "Text" }, products: { html: null } },
    { source: { text: "Text", id: null }, products: { html: true } },
    { source: { text: "Text" }, products: { html: { activeUrls: { unknown: true } } } },
    { source: { text: "Text" }, products: { html: true }, resources: { unknown: true } },
  ]) {
    assert.equal(wasmError(() => wasm.analyze(request)).code, "invalid-request");
  }
});

test("generated wasm-bindgen rejects unknown fields in every request object", () => {
  const unknown = { unknown: true };
  const failedReference = {
    sourceStart: 0,
    sourceEnd: 1,
    outcome: { status: "failed", kind: "missing-target" },
  };
  const resolvedReference = {
    sourceStart: 0,
    sourceEnd: 1,
    outcome: { status: "resolved", href: "https://example.test", notices: [] },
  };
  const failedAsset = {
    sourceStart: 0,
    sourceEnd: 1,
    outcome: { status: "failed", kind: "missing" },
  };
  const resolvedAsset = {
    sourceStart: 0,
    sourceEnd: 1,
    outcome: { status: "resolved", href: "https://example.test/image.png", mediaType: "image/png" },
  };
  const failedCitation = {
    sourceStart: 0,
    sourceEnd: 1,
    outcome: { status: "failed", kind: "missing-target" },
  };
  const resolvedCitation = {
    sourceStart: 0,
    sourceEnd: 1,
    outcome: { status: "resolved", segments: [{ text: "citation" }] },
  };
  const request = (products, resources) => ({
    source: { text: "Text" },
    products,
    ...(resources === undefined ? {} : { resources }),
  });
  const cases = [
    ["AnalyzeRequest", { ...request({ html: true }), ...unknown }],
    ["SourceInput", { ...request({ html: true }), source: { text: "Text", ...unknown } }],
    ["ProductRequest", request({ html: true, ...unknown })],
    ["DiagnosticOptions", request({ diagnostics: { ...unknown } })],
    ["AuthoredUrlOptions", request({ diagnostics: { authoredUrls: { ...unknown } } })],
    ["RuleOptions", request({ diagnostics: { rules: { rule: { ...unknown } } } })],
    ["HtmlOptions", request({ html: { ...unknown } })],
    ["ActiveUrlOptions", request({ html: { activeUrls: { ...unknown } } })],
    ["ExternalLinkOptions", request({ html: { externalLinks: { ...unknown } } })],
    ["SourceLanguageOptions", request({ html: { sourceLanguages: { ...unknown } } })],
    ["RoleOptions", request({ html: { roles: { ...unknown } } })],
    ["ResourceCapabilities", request({ html: { resourceCapabilities: { ...unknown } } })],
    ["Stylesheet.external", request({ html: { stylesheets: [{ kind: "external", url: "https://example.test/a.css", ...unknown }] } })],
    ["Stylesheet.inline", request({ html: { stylesheets: [{ kind: "inline", css: "", ...unknown }] } })],
    ["ResourceInput", request({ html: true }, { ...unknown })],
    ["ResolvedReference", request({ html: true }, { references: [{ ...failedReference, ...unknown }] })],
    ["ReferenceOutcome.resolved", request({ html: true }, { references: [{ ...resolvedReference, outcome: { ...resolvedReference.outcome, ...unknown } }] })],
    ["ReferenceOutcome.failed", request({ html: true }, { references: [{ ...failedReference, outcome: { ...failedReference.outcome, ...unknown } }] })],
    ["ResolvedResource", request({ html: true }, { assets: [{ ...failedAsset, ...unknown }] })],
    ["ResourceOutcome.resolved", request({ html: true }, { assets: [{ ...resolvedAsset, outcome: { ...resolvedAsset.outcome, ...unknown } }] })],
    ["ResourceOutcome.failed", request({ html: true }, { assets: [{ ...failedAsset, outcome: { ...failedAsset.outcome, ...unknown } }] })],
    ["ResolvedCitation", request({ html: true }, { citations: [{ ...failedCitation, ...unknown }] })],
    ["CitationOutcome.resolved", request({ html: true }, { citations: [{ ...resolvedCitation, outcome: { ...resolvedCitation.outcome, ...unknown } }] })],
    ["CitationOutcome.failed", request({ html: true }, { citations: [{ ...failedCitation, outcome: { ...failedCitation.outcome, ...unknown } }] })],
    ["CitationSegment", request({ html: true }, { citations: [{ ...resolvedCitation, outcome: { ...resolvedCitation.outcome, segments: [{ text: "citation", ...unknown }] } }] })],
    ["GeneratedBibliography", request({ html: true }, { bibliography: { title: "References", entries: [], ...unknown } })],
    ["GeneratedBibliographyEntry", request({ html: true }, { bibliography: { title: "References", entries: [{ citationKey: "key", text: "entry", ...unknown }] } })],
  ];
  for (const [name, value] of cases) {
    assert.equal(wasmError(() => wasm.analyze(value)).code, "invalid-request", name);
  }
});

test("generated wasm-bindgen rejects oversized values hidden under unknown fields", () => {
  const unknown = {
    ignoredItems: Array.from({ length: 10_001 }, () => "x"),
    ignoredText: "x".repeat(10 * 1024 * 1024 + 1),
  };
  const request = {
    source: { text: "Text" },
    products: { symbols: true },
    resources: {
      references: [{
        sourceStart: 0,
        sourceEnd: 1,
        outcome: { status: "failed", kind: "missing-target" },
        ...unknown,
      }],
    },
  };
  assert.equal(wasmError(() => wasm.analyze(request)).code, "invalid-request");
});

test("generated wasm-bindgen rejects JavaScript-only invalid values", () => {
  for (const text of [Number.NaN, Number.POSITIVE_INFINITY, () => "Text", Symbol("Text")]) {
    assert.equal(wasmError(() => wasm.analyze({
      source: { text }, products: { html: true },
    })).code, "invalid-request");
  }
  const cyclic = { source: { text: "Text" }, products: { html: true } };
  cyclic.resources = cyclic;
  assert.equal(wasmError(() => wasm.analyze(cyclic)).code, "invalid-request");
});

test("generated wasm-bindgen preserves undefined as omission and rejects null", () => {
  const response = wasm.analyze({
    source: { text: "Text", id: undefined, attributes: undefined },
    products: { html: true, diagnostics: { protectedAttributes: undefined } },
    resources: { bibliography: null },
  });
  assert.equal(typeof response.html, "string");
  assert.equal(typeof wasm.analyze({
    source: { text: "Text", attributes: { set: "value", unset: null } },
    products: { diagnostics: { protectedAttributes: { locked: null } } },
  }).diagnostics, "object");
  assert.equal(wasmError(() => wasm.analyze({
    source: { text: "Text" }, products: { html: true }, resources: null,
  })).code, "invalid-request");
  for (const request of [
    {
      source: { text: "Text", attributes: { invalid: undefined } },
      products: { html: true },
    },
    {
      source: { text: "Text" },
      products: { diagnostics: { protectedAttributes: { invalid: undefined } } },
    },
  ]) {
    assert.equal(wasmError(() => wasm.analyze(request)).code, "invalid-request");
  }
});

test("generated wasm-bindgen applies null protected attributes", () => {
  const diagnostics = wasm.analyze({
    source: { text: ":locked: changed\n" },
    products: { diagnostics: { protectedAttributes: { locked: null } } },
  }).diagnostics;
  assert.equal(diagnostics.some(({ code }) => code === "protected-attribute"), true);
});

test("generated wasm-bindgen aligns omitted render defaults with optional TypeScript fields", () => {
  const base = { source: { text: "Text" }, products: { html: true } };
  const resolvedAsset = (outcome) => ({
    sourceStart: 0, sourceEnd: 1, outcome,
  });
  for (const outcome of [
    { status: "resolved", href: "https://example.test", notices: [] },
    { status: "resolved", href: "https://example.test", notices: undefined },
    { status: "resolved", href: "https://example.test" },
  ]) {
    assert.equal(typeof wasm.analyze({
      ...base, resources: { references: [resolvedAsset(outcome)] },
    }).html, "string");
  }
  for (const byteLength of [undefined, 42]) {
    const outcome = {
      status: "resolved",
      href: "https://example.test/image.png",
      mediaType: "image/png",
      ...(byteLength === undefined ? {} : { byteLength }),
    };
    assert.equal(typeof wasm.analyze({
      ...base, resources: { assets: [resolvedAsset(outcome)] },
    }).html, "string");
  }
  assert.equal(typeof wasm.analyze({
    ...base,
    resources: {
      citations: [resolvedAsset({ status: "resolved" })],
      bibliography: { title: "References" },
    },
  }).html, "string");
  for (const resources of [
    {
      citations: [resolvedAsset({ status: "resolved", segments: undefined })],
    },
    {
      bibliography: { title: "References", entries: undefined },
    },
  ]) {
    assert.equal(typeof wasm.analyze({ ...base, resources }).html, "string");
  }

  const invalidResources = [
    { references: [resolvedAsset({ status: "resolved", href: "https://example.test", displayText: null })] },
    { references: [resolvedAsset({ status: "resolved", href: "https://example.test", notices: null })] },
    { assets: [resolvedAsset({ status: "resolved", href: "https://example.test/a", mediaType: "text/plain", byteLength: null })] },
    { assets: [resolvedAsset({ status: "resolved", href: "https://example.test/a", mediaType: "text/plain", byteLength: 42n })] },
    { citations: [resolvedAsset({ status: "resolved", segments: null })] },
    { citations: [resolvedAsset({ status: "resolved", segments: [{ text: "citation", anchor: null }] })] },
    { bibliography: { title: "References", entries: null } },
    { bibliography: { title: "References", entries: [{ citationKey: "key", text: "entry", label: null }] } },
    { bibliography: { title: "References", entries: [{ citationKey: "key", text: "entry", number: null }] } },
  ];
  for (const resources of invalidResources) {
    assert.equal(wasmError(() => wasm.analyze({ ...base, resources })).code, "invalid-request");
  }
});

test("generated wasm-bindgen accepts only plain data objects and arrays", () => {
  const valid = { source: { text: "Text" }, products: { symbols: true } };
  for (const value of [
    new Date(0),
    new Map(),
    new Set(),
    new Uint8Array(0),
    new Uint8Array([1]),
  ]) {
    assert.equal(wasmError(() => wasm.analyze({ ...valid, resources: value })).code, "invalid-request");
  }

  let getterCalls = 0;
  const accessor = { ...valid };
  Object.defineProperty(accessor, "resources", {
    enumerable: true,
    get() {
      getterCalls += 1;
      throw new Error("getter must not run");
    },
  });
  assert.equal(wasmError(() => wasm.analyze(accessor)).code, "invalid-request");
  assert.equal(getterCalls, 0);

  const nullPrototypeRequest = Object.assign(Object.create(null), {
    source: Object.assign(Object.create(null), { text: "Text" }),
    products: Object.assign(Object.create(null), { symbols: true }),
  });
  assert.deepEqual(wasm.analyze(nullPrototypeRequest).symbols, []);

  const symbolProperty = { ...valid };
  symbolProperty[Symbol("unknown")] = true;
  assert.equal(wasmError(() => wasm.analyze(symbolProperty)).code, "invalid-request");
  const customArray = [];
  customArray.extra = true;
  assert.equal(wasmError(() => wasm.analyze({
    ...valid, resources: { references: customArray },
  })).code, "invalid-request");
});

test("public validation rejects accessors before the actual WASM boundary", () => {
  let getterCalls = 0;
  const request = { source: { text: "Text" }, products: { symbols: true } };
  Object.defineProperty(request, "resources", {
    enumerable: true,
    get() {
      getterCalls += 1;
      return {};
    },
  });
  assert.equal(wasmError(() => wasm.analyze(request)).code, "invalid-request");
  assert.equal(getterCalls, 0);
  assert.throws(
    () => wasm.analyze(analysisPayload(request)),
    (error) => error.code === "invalid-request",
  );
  assert.equal(getterCalls, 0);
});

test("generated wasm-bindgen catches every reflection failure as invalid-request", () => {
  const request = { source: { text: "Text" }, products: { symbols: true } };
  const revoked = Proxy.revocable(request, {});
  revoked.revoke();
  const cases = [
    new Proxy(request, { ownKeys() { throw new Error("ownKeys failed"); } }),
    new Proxy(request, { getPrototypeOf() { throw new Error("getPrototypeOf failed"); } }),
    new Proxy(request, {
      getOwnPropertyDescriptor() { throw new Error("getOwnPropertyDescriptor failed"); },
    }),
    revoked.proxy,
  ];
  for (const value of cases) {
    const error = wasmError(() => wasm.analyze(value));
    assert.equal(error.code, "invalid-request");
    assert.equal(error.constructor, Object);
  }
});

test("generated wasm-bindgen snapshots descriptor values without a second Proxy read", () => {
  const request = { source: { text: "Text" }, products: { symbols: true } };
  let reads = 0;
  const changing = new Proxy(request, {
    get(target, key, receiver) {
      if (key === "source") {
        reads += 1;
        return reads === 1 ? target.source : { text: Symbol("changed") };
      }
      return Reflect.get(target, key, receiver);
    },
  });
  assert.deepEqual(wasm.analyze(changing).symbols, []);
  assert.equal(reads, 0);
});

test("public snapshot fixes a nested value before a later Proxy descriptor mutates it", () => {
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
  assert.deepEqual(wasm.analyze(analysisPayload(request)).symbols, []);
  assert.equal(getterCalls, 0);
});

test("generated wasm-bindgen creates snapshots without inherited setters", () => {
  const request = { source: { text: "Text" }, products: { symbols: true } };
  const names = ["source", "value", "writable", "enumerable", "configurable", "get", "set"];
  let setterCalls = 0;
  for (const name of names) {
    const previous = Object.getOwnPropertyDescriptor(Object.prototype, name);
    try {
      Object.defineProperty(Object.prototype, name, {
        configurable: true,
        set() {
          setterCalls += 1;
          throw new Error("inherited setter must not run");
        },
      });
      assert.deepEqual(wasm.analyze(analysisPayload(request)).symbols, []);
    } finally {
      if (previous === undefined) delete Object.prototype[name];
      else Object.defineProperty(Object.prototype, name, previous);
    }
  }
  assert.equal(setterCalls, 0);
});

test("generated wasm-bindgen rejects prototype-like unknown fields without changing identity", () => {
  const request = JSON.parse(
    '{"source":{"text":"Text"},"products":{"symbols":true},"__proto__":{"large":"value"}}',
  );
  assert.equal(Object.hasOwn(request, "__proto__"), true);
  assert.equal(wasmError(() => wasm.analyze(request)).code, "invalid-request");
});

test("generated wasm-bindgen applies boundary limits before sparse arrays and strings are copied", () => {
  const sparse = new Array(20_001);
  assert.equal(wasmError(() => wasm.analyze({
    source: { text: "Text" },
    products: { symbols: true },
    resources: { references: sparse },
  })).code, "input-limit-exceeded");

  const oversized = "x".repeat(16 * 1024 * 1024 + 1);
  assert.equal(wasmError(() => wasm.analyze({
    source: { text: oversized }, products: { symbols: true },
  })).code, "input-limit-exceeded");

  const tooManyProperties = {};
  for (let index = 0; index < 20_001; index += 1) tooManyProperties[`key${index}`] = true;
  assert.equal(wasmError(() => wasm.analyze({
    source: { text: "Text" }, products: { symbols: true }, unknown: tooManyProperties,
  })).code, "input-limit-exceeded");
});

test("reflection errors do not stop a Worker using the same WASM instance", async (t) => {
  const worker = new Worker(`
    const { parentPort, workerData } = require("node:worker_threads");
    const wasm = require(workerData.wasmModule);
    parentPort.on("message", ({ requestId, kind, payload }) => {
      const rssBefore = process.memoryUsage().rss;
      let request = payload;
      if (kind === "ownKeysProxy") {
        request = new Proxy(payload, { ownKeys() { throw new Error("ownKeys failed"); } });
      } else if (kind === "hugeUnknownKey") {
        request = { source: { text: "Text" }, products: { symbols: true } };
        request["x".repeat(16 * 1024 * 1024)] = true;
      } else if (kind === "changingProxy") {
        let reads = 0;
        request = new Proxy(payload, {
          get(target, key, receiver) {
            if (key === "source") {
              reads += 1;
              return reads === 1 ? target.source : { text: Symbol("changed") };
            }
            return Reflect.get(target, key, receiver);
          },
        });
      }
      try {
        parentPort.postMessage({
          requestId, ok: true, value: wasm.analyze(request), rssDelta: process.memoryUsage().rss - rssBefore,
        });
      } catch (error) {
        parentPort.postMessage({
          requestId,
          ok: false,
          error: { code: error?.code, message: error?.message },
          rssDelta: process.memoryUsage().rss - rssBefore,
        });
      }
    });
  `, { eval: true, workerData: { wasmModule } });
  t.after(() => worker.terminate());
  let nextRequestId = 0;
  const run = (kind, payload) => new Promise((resolveResponse, rejectResponse) => {
    const requestId = nextRequestId += 1;
    const onMessage = (response) => {
      if (response.requestId !== requestId) return;
      worker.off("error", onError);
      resolveResponse(response);
    };
    const onError = (error) => {
      worker.off("message", onMessage);
      rejectResponse(error);
    };
    worker.once("message", onMessage);
    worker.once("error", onError);
    worker.postMessage({ requestId, kind, payload });
  });

  const invalid = await run("ownKeysProxy", {
    source: { text: "Text" }, products: { symbols: true },
  });
  assert.equal(invalid.ok, false);
  assert.equal(invalid.error.code, "invalid-request");
  const hugeKey = await run("hugeUnknownKey");
  assert.equal(hugeKey.ok, false);
  assert.equal(hugeKey.error.code, "input-limit-exceeded");
  assert.ok(hugeKey.error.message.length < 256);
  assert.ok(hugeKey.rssDelta < 128 * 1024 * 1024);
  const changing = await run("changingProxy", {
    source: { text: "Text" }, products: { symbols: true },
  });
  assert.equal(changing.ok, true);
  assert.deepEqual(changing.value.symbols, []);
  const valid = await run("plain", {
    source: { text: "Text" }, products: { symbols: true },
  });
  assert.equal(valid.ok, true);
  assert.deepEqual(valid.value.symbols, []);
});

test("generated wasm-bindgen reports UTF-8 byte ranges", () => {
  const response = wasm.analyze({
    source: { text: "= 文書\n" },
    products: { symbols: true },
  });
  assert.equal(response.symbols[0].range.end, 9);
});

test("generated wasm-bindgen enforces external input limits and range conflicts", () => {
  const documents = Object.fromEntries(
    Array.from({ length: 10_001 }, (_, index) => [`${index}.adoc`, ""]),
  );
  assert.equal(wasmError(() => wasm.analyze({
    source: { text: "Text" },
    products: { symbols: true },
    resources: { documents },
  })).code, "input-limit-exceeded");

  const failed = {
    sourceStart: 0,
    sourceEnd: 4,
    outcome: { status: "failed", kind: "missing-target" },
  };
  assert.equal(wasmError(() => wasm.analyze({
    source: { text: "Text" },
    products: { symbols: true },
    resources: { references: [failed, failed] },
  })).code, "invalid-request");
});
