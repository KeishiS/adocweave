import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { PROTOCOL_SCHEMA_VERSION } from "./worker-protocol.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const require = createRequire(import.meta.url);
const wasm = require(resolve(root, "target/adocweave-wasm-node/adocweave_wasm.js"));

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
    source: { text: "Text", id: undefined },
    products: { html: true },
    resources: undefined,
  });
  assert.equal(typeof response.html, "string");
  assert.equal(wasmError(() => wasm.analyze({
    source: { text: "Text" }, products: { html: true }, resources: null,
  })).code, "invalid-request");
});

test("generated wasm-bindgen reports UTF-8 byte ranges", () => {
  const response = wasm.analyze({
    source: { text: "= 文書\n" },
    products: { symbols: true },
  });
  assert.equal(response.symbols[0].range.end, 9);
});
