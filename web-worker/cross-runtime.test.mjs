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

for (const [name, responsibility] of [
  ["attributes-anchors-links-references-lists-stem", "全productの変換"],
  ["position-dependent-attribute-queries-with-include-origin", "include前処理"],
  ["resolved-render-inputs", "解決済み描画入力"],
  ["strict-unsupported", "入力errorの変換"],
]) {
  const entry = manifest.cases.find((candidate) => candidate.name === name);
  if (!entry) throw new Error(`cross-runtime case is missing: ${name}`);
  test(`nativeとWASMで${responsibility}が一致する`, () => {
    const request = requestFor(entry);
    const expected = nativeResult(request);
    const actual = wasmResult(request);
    assert.deepEqual(actual, expected);
  });
}
