import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const fixtureRoot = resolve(root, "fixtures/conformance");
const manifest = JSON.parse(
  readFileSync(resolve(root, "crates/adocweave/conformance/cases.json"), "utf8"),
);
const require = createRequire(import.meta.url);
const wasm = require(resolve(root, "target/adocweave-wasm-node/adocweave_wasm.js"));
const native = resolve(root, "target/debug/adocweave-conformance-native");

function requestFor(entry) {
  const source = entry.sourceFile
    ? readFileSync(resolve(fixtureRoot, entry.sourceFile), "utf8")
    : entry.source;
  return {
    sourceId: entry.sourceId ?? `conformance:${entry.name}`,
    source,
    preprocess: entry.preprocess ?? null,
    products: {
      syntax: true,
      canonicalAst: true,
      html: true,
      attributeOccurrences: true,
      attributeQueries: true,
      resourceQueries: true,
      diagnostics: true,
      symbols: true,
      projection: true,
    },
    renderInputs: entry.renderInputs ?? {},
    analysisOptions: entry.analysisOptions ?? {},
    renderPolicy: entry.renderPolicy ?? {},
    outputLimits: entry.outputLimits ?? {},
  };
}

function nativeResult(request) {
  const run = spawnSync(native, [], {
    cwd: root,
    input: `${JSON.stringify(request)}\n`,
    encoding: "utf8",
  });
  assert.equal(run.status, 0, run.stderr);
  return JSON.parse(run.stdout);
}

function wasmResult(request) {
  try {
    return { ok: true, value: wasm.process(request) };
  } catch (error) {
    const text = String(error);
    let value;
    try {
      value = JSON.parse(text);
    } catch {
      value = text;
    }
    return { ok: false, error: value };
  }
}

function expectedProduct(entry, inlineField, fileField, readFile) {
  const hasInline = Object.hasOwn(entry, inlineField);
  const hasFile = Object.hasOwn(entry, fileField);
  assert.equal(
    hasInline && hasFile,
    false,
    `${entry.name}: ${inlineField} and ${fileField} are mutually exclusive`,
  );
  if (hasInline) return { present: true, value: entry[inlineField] };
  if (hasFile) {
    return {
      present: true,
      value: readFile(resolve(fixtureRoot, entry[fileField])),
    };
  }
  return { present: false };
}

test("inline期待値は空配列を保持しfile期待値との重複を拒否する", () => {
  const inline = expectedProduct(
    { name: "inline-empty", expectedDiagnostics: [] },
    "expectedDiagnostics",
    "expectedDiagnosticsFile",
    readJson,
  );
  assert.deepEqual(inline, { present: true, value: [] });
  assert.throws(
    () => expectedProduct(
      {
        name: "duplicate-expectation",
        expectedDiagnostics: [],
        expectedDiagnosticsFile: "diagnostics.json",
      },
      "expectedDiagnostics",
      "expectedDiagnosticsFile",
      readJson,
    ),
    /mutually exclusive/,
  );
});

for (const entry of manifest.cases) {
  test(`native and WASM agree: ${entry.name}`, () => {
    const request = requestFor(entry);
    const expected = nativeResult(request);
    const actual = wasmResult(request);
    assert.deepEqual(actual, expected);

    for (const [inlineField, fileField, product, readFile] of [
      [
        "expectedHtml",
        "expectedHtmlFile",
        "html",
        (path) => readFileSync(path, "utf8"),
      ],
      [
        "expectedAst",
        "expectedAstFile",
        "ast",
        (path) => readFileSync(path, "utf8").trimEnd(),
      ],
      ["expectedDiagnostics", "expectedDiagnosticsFile", "diagnostics", readJson],
      [
        "expectedRenderDiagnostics",
        "expectedRenderDiagnosticsFile",
        "renderDiagnostics",
        readJson,
      ],
      ["expectedProjection", "expectedProjectionFile", "projection", readJson],
      ["expectedSymbols", "expectedSymbolsFile", "symbols", readJson],
    ]) {
      const expectedProductValue = expectedProduct(
        entry,
        inlineField,
        fileField,
        readFile,
      );
      if (expectedProductValue.present) {
        assert.deepEqual(actual.value[product], expectedProductValue.value);
      }
    }
    if (entry.expectedErrorCode) {
      assert.equal(actual.error.code, entry.expectedErrorCode);
    }
  });
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}
