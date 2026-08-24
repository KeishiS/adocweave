import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";

import { runBrowserNpmSmoke } from "./browser-npm-smoke.mjs";

const manifest = {
  name: "@adocweave/browser",
  version: "0.47.0",
  exports: { ".": { types: "./worker/index.d.mts", import: "./worker/index.mjs" } }
};

const PACKAGE_FILES = [
  "worker/index.mjs",
  "worker/index.d.mts",
  "worker/worker.mjs",
  "wasm/adocweave_wasm.js",
  "wasm/adocweave_wasm_bg.wasm",
  "README.md",
  "THIRD_PARTY_NOTICES.adoc"
];

async function fakeInstall({ cwd }, overrides = {}) {
  const packageRoot = join(cwd, "node_modules", "@adocweave", "browser");
  await mkdir(join(packageRoot, "worker"), { recursive: true });
  await mkdir(join(packageRoot, "wasm"), { recursive: true });
  await writeFile(
    join(packageRoot, "package.json"),
    JSON.stringify({ ...manifest, type: "module", ...overrides })
  );
  for (const path of PACKAGE_FILES) await writeFile(join(packageRoot, path), "\n");
}

test("registryのversionを解決してから導入した内容を確かめる", async () => {
  const requested = [];
  const published = await runBrowserNpmSmoke({
    manifest,
    fetchJson: async (url) => {
      requested.push(url);
      return { name: manifest.name, version: manifest.version };
    },
    install: fakeInstall
  });
  assert.deepEqual(published, { name: manifest.name, version: manifest.version });
  assert.deepEqual(requested, ["https://registry.npmjs.org/@adocweave/browser/0.47.0"]);
});

test("公開前に実行した場合は導入を試さず止める", async () => {
  let installed = false;
  await assert.rejects(
    runBrowserNpmSmoke({
      manifest,
      fetchJson: async () => ({ error: "Not found" }),
      install: async () => { installed = true; }
    }),
    /公開のあとに実行してください/u
  );
  assert.equal(installed, false);
});

test("公開entryの宣言が違う導入を拒否する", async () => {
  await assert.rejects(
    runBrowserNpmSmoke({
      manifest,
      fetchJson: async () => ({ name: manifest.name, version: manifest.version }),
      install: async (options) => fakeInstall(options, { exports: { ".": "./index.mjs" } })
    }),
    /公開entryの宣言が一致しません/u
  );
});

test("実行時npm依存を持つ導入を拒否する", async () => {
  await assert.rejects(
    runBrowserNpmSmoke({
      manifest,
      fetchJson: async () => ({ name: manifest.name, version: manifest.version }),
      install: async (options) => fakeInstall(options, { dependencies: { left: "1.0.0" } })
    }),
    /実行時npm依存を持ちません/u
  );
});
