import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";
import {
  PROTOCOL_SCHEMA_VERSION,
  PACKAGE_VERSION,
  validateWorkerMessage,
  WORKER_PROTOCOL_VERSION,
} from "./worker-protocol.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const require = createRequire(import.meta.url);
const wasm = require(resolve(root, "target/adocweave-wasm-node/adocweave_wasm.js"));
const corpus = JSON.parse(
  readFileSync(resolve(root, "fixtures/protocol/request-corpus.json"), "utf8"),
);

function currentRequest() {
  return {
    ...structuredClone(corpus.defaultRequest),
    packageVersion: PACKAGE_VERSION,
  };
}

function currentPreprocessRequest() {
  return {
    ...structuredClone(corpus.preprocessRequest),
    packageVersion: PACKAGE_VERSION,
  };
}

function wasmError(operation) {
  try {
    operation();
  } catch (error) {
    return JSON.parse(String(error));
  }
  assert.fail("WASM request unexpectedly succeeded");
}

test("generated wasm-bindgen accepts the current default requests", () => {
  assert.equal(wasm.protocolSchemaVersion(), PROTOCOL_SCHEMA_VERSION);
  const response = wasm.process(currentRequest());
  assert.equal(response.packageVersion, PACKAGE_VERSION);
  assert.equal(response.version, 1);
  assert.equal(response.generation, 1);
  assert.equal(validateWorkerMessage({
    protocolVersion: WORKER_PROTOCOL_VERSION,
    type: "result",
    version: response.version,
    generation: response.generation,
    result: response,
  }, "responses"), true);

  const withoutProjection = currentRequest();
  withoutProjection.products = { ...response.products, projection: false };
  const disabledResponse = wasm.process(withoutProjection);
  assert.equal(disabledResponse.projection, null);
  assert.equal(validateWorkerMessage({
    protocolVersion: WORKER_PROTOCOL_VERSION,
    type: "result",
    version: disabledResponse.version,
    generation: disabledResponse.generation,
    result: disabledResponse,
  }, "responses"), true);

  const preprocessed = wasm.preprocess(currentPreprocessRequest());
  assert.equal(preprocessed.packageVersion, PACKAGE_VERSION);
  assert.equal(preprocessed.source, "included\n");
});

test("generated wasm-bindgen returns typed products and explicit disabled sentinels", () => {
  const enabled = currentRequest();
  enabled.source = "= Title\n\nxref:missing[] \n";
  enabled.analysisOptions = structuredClone(corpus.defaultRequestExpansion.analysisOptions);
  enabled.analysisOptions.diagnostics.rules["trailing-whitespace"] = {
    enabled: true,
    severity: "warning",
  };
  enabled.products = Object.fromEntries(
    Object.keys(corpus.browserProductDefault).map((name) => [name, true]),
  );
  const response = wasm.process(enabled);

  assert.match(response.syntax, /Document/);
  assert.equal(JSON.parse(response.ast).schemaVersion, 2);
  assert.match(response.html, /<h1[^>]*>Title<\/h1>/);
  assert.ok(response.diagnostics.length > 0);
  assert.ok(response.renderDiagnostics.length > 0);
  assert.equal(response.symbols[0].name, "Title");
  assert.equal(response.projection.title.text, "Title");
  assert.equal(response.projection.structure.headings[0].title, "Title");
  assert.ok(Array.isArray(response.attributeOccurrences));
  assert.ok(Array.isArray(response.attributeQueries.bindings));
  assert.ok(Array.isArray(response.resourceQueries));

  const disabled = currentRequest();
  disabled.products = Object.fromEntries(
    Object.keys(corpus.browserProductDefault).map((name) => [name, false]),
  );
  const disabledResponse = wasm.process(disabled);
  assert.equal(disabledResponse.syntax, "");
  assert.equal(disabledResponse.ast, "");
  assert.equal(disabledResponse.html, "");
  assert.deepEqual(disabledResponse.attributeOccurrences, []);
  assert.deepEqual(disabledResponse.attributeQueries, { bindings: [], references: [] });
  assert.deepEqual(disabledResponse.resourceQueries, []);
  assert.deepEqual(disabledResponse.diagnostics, []);
  assert.deepEqual(disabledResponse.renderDiagnostics, []);
  assert.deepEqual(disabledResponse.symbols, []);
  assert.equal(disabledResponse.projection, null);
});

test("generated wasm-bindgen measures the exact UTF-8 JSON output boundary", () => {
  const source = "= 日本語 😀\n\n\\\"引用\\\"\n";
  const unrestricted = currentRequest();
  unrestricted.source = source;
  unrestricted.outputLimits = { maxOutputBytes: 0xffff_ffff };
  const response = wasm.process(unrestricted);
  const exactBytes = Buffer.byteLength(JSON.stringify(response), "utf8");

  const exact = currentRequest();
  exact.source = source;
  exact.outputLimits = { maxOutputBytes: exactBytes };
  wasm.process(exact);

  const tooSmall = currentRequest();
  tooSmall.source = source;
  tooSmall.outputLimits = { maxOutputBytes: exactBytes - 1 };
  const error = wasmError(() => wasm.process(tooSmall));
  assert.equal(error.code, "limit-exceeded");
  assert.equal(
    error.message,
    `output bytes limit exceeded (limit ${exactBytes - 1}, actual ${exactBytes})`,
  );
});

test("generated wasm-bindgen rejects unknown and missing fields", () => {
  for (const { path, value } of corpus.unknownFieldCases) {
    const request = currentRequest();
    setPointer(request, path, value);
    assert.equal(wasmError(() => wasm.process(request)).code, "invalid-request", path);
  }

  for (const field of ["packageVersion", "version", "generation", "source"]) {
    const request = currentRequest();
    delete request[field];
    assert.equal(wasmError(() => wasm.process(request)).code, "invalid-request", field);
  }

  const preprocess = currentPreprocessRequest();
  preprocess.options.unexpected = true;
  assert.equal(wasmError(() => wasm.preprocess(preprocess)).code, "invalid-request");
});

test("generated wasm-bindgen rejects old package versions", () => {
  const request = currentRequest();
  request.packageVersion = corpus.oldVersion;
  assert.equal(
    wasmError(() => wasm.process(request)).code,
    "unsupported-api-version",
  );

  const preprocess = currentPreprocessRequest();
  preprocess.packageVersion = corpus.oldVersion;
  assert.equal(
    wasmError(() => wasm.preprocess(preprocess)).code,
    "unsupported-api-version",
  );
});

function setPointer(rootValue, pointer, value) {
  const segments = pointer.split("/").slice(1).map((segment) =>
    segment.replaceAll("~1", "/").replaceAll("~0", "~")
  );
  const key = segments.pop();
  let target = rootValue;
  for (const segment of segments) {
    target[segment] ??= {};
    target = target[segment];
  }
  target[key] = structuredClone(value);
}
