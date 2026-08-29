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
