import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

import { npmInvocation } from "./textlint-plugin-consumer-e2e.mjs";

const MANIFEST = new URL("../web-worker/package.json", import.meta.url);

// 公開したBrowser packageをnpm Registryから導入し直し、registryのversion解決を
// 経た取得と、公開entryおよび型定義の解決を観測する。実browserでのWorkerとWASMの
// URL解決は、同じbyte列に対して公開前のgateで確認済み。
export async function runBrowserNpmSmoke({
  manifest,
  fetchJson = defaultFetchJson,
  install = installFromRegistry
} = {}) {
  manifest ??= JSON.parse(await readFile(MANIFEST, "utf8"));
  const { name, version } = manifest;
  if (typeof version !== "string") {
    throw new Error("Browser packageのversionを解釈できません");
  }
  const metadata = await fetchJson(`https://registry.npmjs.org/${name}/${version}`);
  if (metadata?.name !== name || metadata?.version !== version) {
    throw new Error(
      `npmに${name}@${version}が見つかりません。公開後smokeは公開のあとに実行してください。`
    );
  }
  const root = await mkdtemp(join(tmpdir(), "adocweave-browser-npm-smoke-"));
  try {
    await install({ spec: `${name}@${version}`, cwd: root });
    await assertInstalledPackage(root, manifest);
    return Object.freeze({ name, version });
  } finally {
    await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}

async function assertInstalledPackage(root, expected) {
  const packageRoot = join(root, "node_modules", ...expected.name.split("/"));
  const manifest = JSON.parse(await readFile(join(packageRoot, "package.json"), "utf8"));
  assert.equal(manifest.name, expected.name);
  assert.equal(manifest.version, expected.version);
  assert.deepEqual(manifest.exports, expected.exports, "公開entryの宣言が一致しません");
  assert.equal(manifest.dependencies, undefined, "Browser packageは実行時npm依存を持ちません");

  // filesへ挙げたentryが実際に届いていることを、公開後の導入で確かめる。
  for (const path of [
    "worker/index.mjs",
    "worker/index.d.mts",
    "worker/worker.mjs",
    "wasm/adocweave_wasm.js",
    "wasm/adocweave_wasm_bg.wasm",
    "README.md",
    "THIRD_PARTY_NOTICES.adoc"
  ]) {
    await readFile(join(packageRoot, path));
  }

  // exports mapを経た解決が公開entryへ届くことを、利用側と同じ経路で確かめる。
  const resolved = await runNode(
    ["--input-type=module", "-e", `process.stdout.write(import.meta.resolve(${JSON.stringify(expected.name)}))`],
    root
  );
  assert.equal(
    resolved.trim(),
    pathToFileURL(join(packageRoot, "worker", "index.mjs")).href,
    "exports mapが公開entryへ解決しません"
  );
}

async function installFromRegistry({ spec, cwd }) {
  await writeFile(
    join(cwd, "package.json"),
    `${JSON.stringify({ name: "adocweave-browser-npm-smoke", private: true, type: "module" }, null, 2)}\n`
  );
  const npm = npmInvocation();
  const result = await runProcess(npm.command, [
    ...npm.arguments,
    "install",
    "--ignore-scripts",
    "--no-audit",
    "--no-fund",
    "--prefer-online",
    spec
  ], { cwd, env: { ...process.env, npm_config_cache: join(cwd, ".npm-cache") } });
  if (result.code !== 0) {
    throw new Error(`npm install exited with ${result.code}\n${result.stdout}\n${result.stderr}`);
  }
}

async function runNode(args, cwd) {
  const result = await runProcess(process.execPath, args, { cwd });
  if (result.code !== 0) {
    throw new Error(`node exited with ${result.code}\n${result.stdout}\n${result.stderr}`);
  }
  return result.stdout;
}

function runProcess(command, args, { cwd, env = process.env } = {}) {
  return new Promise((resolveProcess, rejectProcess) => {
    const child = spawn(command, args, { cwd, env, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => { stdout += chunk; });
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    child.on("error", rejectProcess);
    child.on("close", (code) => resolveProcess({ code, stdout, stderr }));
  });
}

async function defaultFetchJson(url) {
  const response = await fetch(url, { headers: { accept: "application/json" } });
  if (!response.ok) {
    throw new Error(`npm registryへの問い合わせに失敗しました: HTTP ${response.status}`);
  }
  return response.json();
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  const published = await runBrowserNpmSmoke();
  process.stdout.write(`browser npm smoke passed: ${published.name}@${published.version}\n`);
}
