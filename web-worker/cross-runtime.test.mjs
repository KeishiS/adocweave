import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const require = createRequire(import.meta.url);
const wasm = require(resolve(root, "target/adocweave-wasm-node/adocweave_wasm.js"));
const native = resolve(root, "target/debug/adocweave-conformance-native");

const cases = [
  {
    name: "要求した全product",
    request: {
      source: { text: "= Title\n\n== Section\n\nText *strong*.\n", id: "main.adoc" },
      products: {
        syntax: true, canonicalAst: true, html: true, attributeOccurrences: true,
        attributeQueries: true, resourceQueries: true, diagnostics: true, symbols: true,
        document: true,
      },
    },
  },
  {
    name: "include文書",
    request: {
      source: { text: "include::part.adoc[]", id: "main.adoc" },
      products: { html: true, attributeQueries: true },
      resources: { documents: { "part.adoc": "== Included\n" } },
    },
  },
  {
    name: "不正な要求",
    request: { source: { text: "Text" }, products: { html: false } },
  },
  {
    name: "入れ子の未知項目を持つ要求",
    request: {
      source: { text: "Text" },
      products: { symbols: true },
      resources: {
        references: [{
          sourceStart: 0,
          sourceEnd: 1,
          outcome: { status: "failed", kind: "missing-target" },
          unknown: Array.from({ length: 10_001 }, () => "ignored"),
        }],
      },
    },
  },
  {
    name: "prototype名の未知項目を持つ要求",
    request: JSON.parse(
      '{"source":{"text":"Text"},"products":{"symbols":true},"__proto__":{"value":1}}',
    ),
  },
  {
    name: "省略可能なrender入力を省略した要求",
    request: {
      source: { text: "Text" },
      products: { html: true },
      resources: {
        references: [{
          sourceStart: 0,
          sourceEnd: 1,
          outcome: { status: "resolved", href: "https://example.test" },
        }],
        assets: [{
          sourceStart: 1,
          sourceEnd: 2,
          outcome: {
            status: "resolved",
            href: "https://example.test/a",
            mediaType: "text/plain",
            byteLength: 42,
          },
        }],
      },
    },
  },
  {
    name: "省略専用のrender入力へnullを指定した要求",
    request: {
      source: { text: "Text" },
      products: { html: true },
      resources: {
        assets: [{
          sourceStart: 0,
          sourceEnd: 1,
          outcome: {
            status: "resolved",
            href: "https://example.test/a",
            mediaType: "text/plain",
            byteLength: null,
          },
        }],
      },
    },
  },
  {
    name: "bibliographyを明示的に解除した要求",
    request: {
      source: { text: "Text" },
      products: { html: true },
      resources: { bibliography: null },
    },
  },
];

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
    return { ok: true, value: wasm.analyze(request) };
  } catch (error) {
    return { ok: false, error };
  }
}

for (const { name, request } of cases) {
  test(`nativeとWASMで${name}が一致する`, () => {
    const actual = wasmResult(request);
    const expected = nativeResult(request);
    assert.equal(actual.ok, expected.ok);
    if (actual.ok) assert.deepEqual(actual.value, expected.value);
    else assert.equal(actual.error.code, expected.error.code);
  });
}
