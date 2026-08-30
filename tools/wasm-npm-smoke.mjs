import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

import { npmInvocation } from "./textlint-plugin-consumer-e2e.mjs";
import { runWasmPackageBrowserSmoke } from "./wasm-release-smoke.mjs";

const MANIFEST = new URL("../packages/wasm/package.json", import.meta.url);

// 公開したWebAssembly packageをnpm Registryから導入し直し、registryのversion解決を
// 経た取得、署名とprovenance、公開entry、Node.jsおよび実ブラウザーでのWorkerとWASMを確認する。
export async function runWasmNpmSmoke({
  manifest,
  fetchJson = defaultFetchJson,
  install = installFromRegistry,
  audit = auditInstalledPackage,
  browser = runInstalledPackageBrowserSmoke,
} = {}) {
  manifest ??= JSON.parse(await readFile(MANIFEST, "utf8"));
  const { name, version } = manifest;
  if (typeof version !== "string") {
    throw new Error("WebAssembly packageのversionを解釈できません");
  }
  const metadata = await fetchJson(`https://registry.npmjs.org/${name}/${version}`);
  if (metadata?.name !== name || metadata?.version !== version) {
    throw new Error(
      `npmに${name}@${version}が見つかりません。公開後smokeは公開のあとに実行してください。`
    );
  }
  const root = await mkdtemp(join(tmpdir(), "adocweave-wasm-npm-smoke-"));
  try {
    await install({ spec: `${name}@${version}`, cwd: root });
    const packageRoot = await assertInstalledPackage(root, manifest);
    await audit({ root, name, version });
    await browser({ packageRoot });
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
  assert.equal(manifest.dependencies, undefined, "WebAssembly packageは実行時npm依存を持ちません");

  // filesへ挙げたentryが実際に届いていることを、公開後の導入で確かめる。
  for (const path of [
    "worker/index.mjs",
    "worker/index.d.mts",
    "worker/direct.mjs",
    "worker/direct.d.mts",
    "worker/worker.mjs",
    "wasm/adocweave_wasm.js",
    "wasm/adocweave_wasm_bg.wasm",
    "README.md",
    "THIRD_PARTY_NOTICES.adoc"
  ]) {
    await readFile(join(packageRoot, path));
  }

  // exports mapを経た解決が公開entryへ届くことを、利用側と同じ経路で確かめる。
  for (const [specifier, entry] of [
    [expected.name, "index.mjs"],
    [`${expected.name}/direct`, "direct.mjs"]
  ]) {
    const resolved = await runNode(
      ["--input-type=module", "-e", `process.stdout.write(import.meta.resolve(${JSON.stringify(specifier)}))`],
      root
    );
    assert.equal(
      resolved.trim(),
      pathToFileURL(join(packageRoot, "worker", entry)).href,
      `exports mapが${specifier}を公開entryへ解決しません`
    );
  }

  // ビルド時の入口が、導入したpackageの同梱WebAssemblyで実際に動くことを確かめる。
  const html = await runNode([
    "--input-type=module",
    "-e",
    `const { analyze } = await import(${JSON.stringify(`${expected.name}/direct`)});
     const result = await analyze({ source: "= 題名\\n" });
     process.stdout.write(String(result.html));`
  ], root);
  assert.match(html, /題名/u, "ビルド時の入口が解析結果を返しません");
  return packageRoot;
}

export function assertSignatureAudit(report, { name, version }) {
  assert.deepEqual(report.invalid, [], "npm署名またはattestationが無効です");
  assert.deepEqual(report.missing, [], "npm Registryの署名がありません");
  const verified = report.verified?.find((entry) =>
    entry.name === name && entry.version === version
  );
  assert.ok(verified, `${name}@${version}のattestationが検証されていません`);
  assert.equal(
    verified.attestations?.provenance?.predicateType,
    "https://slsa.dev/provenance/v1",
    "SLSA provenance v1がありません",
  );
  const provenance = verified.attestationBundles?.find((entry) =>
    entry.predicateType === "https://slsa.dev/provenance/v1"
  );
  assert.ok(provenance?.bundle?.dsseEnvelope?.signatures?.length > 0,
    "署名付きprovenance attestationがありません");
}

async function auditInstalledPackage({ root, name, version }) {
  const npm = npmInvocation();
  const result = await runProcess(npm.command, [
    ...npm.arguments,
    "audit",
    "signatures",
    "--json",
    "--include-attestations",
  ], { cwd: root, env: { ...process.env, npm_config_cache: join(root, ".npm-cache") } });
  if (result.code !== 0) {
    throw new Error(`npm audit signatures exited with ${result.code}\n${result.stdout}\n${result.stderr}`);
  }
  let report;
  try {
    report = JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`npm audit signatures returned invalid JSON: ${error.message}`);
  }
  assertSignatureAudit(report, { name, version });
}

async function runInstalledPackageBrowserSmoke({ packageRoot }) {
  await runWasmPackageBrowserSmoke(
    packageRoot,
    process.env.ADOCWEAVE_BROWSER ?? "chromium",
  );
}

async function installFromRegistry({ spec, cwd }) {
  await writeFile(
    join(cwd, "package.json"),
    `${JSON.stringify({ name: "adocweave-wasm-npm-smoke", private: true, type: "module" }, null, 2)}\n`
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
  const published = await runWasmNpmSmoke();
  process.stdout.write(`wasm npm smoke passed: ${published.name}@${published.version}\n`);
}
