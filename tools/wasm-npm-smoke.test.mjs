import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";

import { assertSignatureAudit, runWasmNpmSmoke } from "./wasm-npm-smoke.mjs";

const manifest = {
  name: "@adocweave/wasm",
  version: "0.47.0",
  exports: {
    ".": { types: "./worker/index.d.mts", import: "./worker/index.mjs" },
    "./direct": { types: "./worker/direct.d.mts", import: "./worker/direct.mjs" }
  }
};

const PACKAGE_FILES = [
  "worker/index.mjs",
  "worker/index.d.mts",
  "worker/direct.d.mts",
  "worker/worker.mjs",
  "wasm/adocweave_wasm.js",
  "wasm/adocweave_wasm_bg.wasm",
  "README.md",
  "THIRD_PARTY_NOTICES.adoc"
];

// 実際の解析は公開後smokeが本物のpackageで確かめる。ここでは手順の組み立てだけを見る。
const DIRECT_STUB = `
export const analyze = async (request) => {
  if (request?.source?.text !== "= 題名\\n" || request?.products?.html !== true) {
    throw new Error("unexpected analysis request");
  }
  return { html: "<h1>題名</h1>" };
};
`;

async function fakeInstall({ cwd }, overrides = {}) {
  const packageRoot = join(cwd, "node_modules", "@adocweave", "wasm");
  await mkdir(join(packageRoot, "worker"), { recursive: true });
  await mkdir(join(packageRoot, "wasm"), { recursive: true });
  await writeFile(
    join(packageRoot, "package.json"),
    JSON.stringify({ ...manifest, type: "module", ...overrides })
  );
  for (const path of PACKAGE_FILES) await writeFile(join(packageRoot, path), "\n");
  await writeFile(join(packageRoot, "worker", "direct.mjs"), DIRECT_STUB);
}

test("registryのversionを解決してから導入した内容を確かめる", async () => {
  const requested = [];
  const observed = [];
  const published = await runWasmNpmSmoke({
    manifest,
    fetchJson: async (url) => {
      requested.push(url);
      return { name: manifest.name, version: manifest.version };
    },
    install: fakeInstall,
    audit: async ({ name, version }) => observed.push(`audit:${name}@${version}`),
    browser: async ({ packageRoot }) => observed.push(`browser:${packageRoot}`),
  });
  assert.deepEqual(published, { name: manifest.name, version: manifest.version });
  assert.deepEqual(requested, ["https://registry.npmjs.org/@adocweave/wasm/0.47.0"]);
  assert.equal(observed[0], "audit:@adocweave/wasm@0.47.0");
  assert.match(observed[1], /browser:.*node_modules.*@adocweave.*wasm/u);
});

test("公開前に実行した場合は導入を試さず止める", async () => {
  let installed = false;
  await assert.rejects(
    runWasmNpmSmoke({
      manifest,
      fetchJson: async () => ({ error: "Not found" }),
      install: async () => { installed = true; },
      audit: async () => {},
      browser: async () => {},
    }),
    /公開のあとに実行してください/u
  );
  assert.equal(installed, false);
});

test("公開entryの宣言が違う導入を拒否する", async () => {
  await assert.rejects(
    runWasmNpmSmoke({
      manifest,
      fetchJson: async () => ({ name: manifest.name, version: manifest.version }),
      install: async (options) => fakeInstall(options, { exports: { ".": "./index.mjs" } }),
      audit: async () => {},
      browser: async () => {},
    }),
    /公開entryの宣言が一致しません/u
  );
});

test("実行時npm依存を持つ導入を拒否する", async () => {
  await assert.rejects(
    runWasmNpmSmoke({
      manifest,
      fetchJson: async () => ({ name: manifest.name, version: manifest.version }),
      install: async (options) => fakeInstall(options, { dependencies: { left: "1.0.0" } }),
      audit: async () => {},
      browser: async () => {},
    }),
    /実行時npm依存を持ちません/u
  );
});

test("対象packageの署名付きSLSA provenanceを要求する", () => {
  const provenance = {
    predicateType: "https://slsa.dev/provenance/v1",
    bundle: { dsseEnvelope: { signatures: [{ sig: "verified" }] } },
  };
  assert.doesNotThrow(() => assertSignatureAudit({
    invalid: [],
    missing: [],
    verified: [{
      name: manifest.name,
      version: manifest.version,
      attestations: { provenance: { predicateType: provenance.predicateType } },
      attestationBundles: [provenance],
    }],
  }, manifest));

  assert.throws(() => assertSignatureAudit({
    invalid: [],
    missing: [],
    verified: [],
  }, manifest), /attestationが検証されていません/u);
  assert.throws(() => assertSignatureAudit({
    invalid: [],
    missing: [],
    verified: [{
      name: manifest.name,
      version: manifest.version,
      attestations: { provenance: { predicateType: provenance.predicateType } },
      attestationBundles: [{ ...provenance, bundle: { dsseEnvelope: { signatures: [] } } }],
    }],
  }, manifest), /署名付きprovenance/u);
});
