import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import process from "node:process";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

import {
  npmInvocation,
  runTextlintPluginConsumerE2E,
} from "./textlint-plugin-consumer-e2e.mjs";
import { runTextlintPluginNpxSmoke } from "./textlint-plugin-npx-smoke.mjs";
import { loadTextlintPluginManifest } from "./textlint-plugin-package.mjs";

// 公開したpackageをnpm Registryから導入し直し、registryのversion解決、署名、
// provenance、固定consumerおよびnpxの経路を確認する。
export async function runTextlintPluginNpmSmoke({
  manifest = loadTextlintPluginManifest(),
  fetchJson = defaultFetchJson,
  audit = auditRegistryPackage,
  runConsumerE2E = runTextlintPluginConsumerE2E,
  runNpxSmoke = runTextlintPluginNpxSmoke
} = {}) {
  const { name, version } = manifest;
  if (typeof version !== "string") {
    throw new Error("Cannot determine the textlint plugin package version");
  }
  const metadata = await fetchJson(`https://registry.npmjs.org/${name}/${version}`);
  if (metadata?.name !== name || metadata?.version !== version) {
    throw new Error(`${name}@${version} is not available from the npm Registry`);
  }
  const spec = `${name}@${version}`;
  await audit({ name, version });
  await runConsumerE2E(spec, { manifest });
  await runNpxSmoke(spec, { manifest });
  return Object.freeze({ name, version });
}

export function assertSignatureAudit(report, { name, version }) {
  assert.deepEqual(report.invalid, [], "npm returned invalid signatures or attestations");
  assert.deepEqual(report.missing, [], "npm returned packages without registry signatures");
  const verified = report.verified?.find((entry) =>
    entry.name === name && entry.version === version
  );
  assert.ok(verified, `${name}@${version} does not have a verified attestation`);
  assert.equal(
    verified.attestations?.provenance?.predicateType,
    "https://slsa.dev/provenance/v1",
    `${name}@${version} does not have SLSA provenance v1`,
  );
  const provenance = verified.attestationBundles?.find((entry) =>
    entry.predicateType === "https://slsa.dev/provenance/v1"
  );
  assert.ok(
    provenance?.bundle?.dsseEnvelope?.signatures?.length > 0,
    `${name}@${version} does not have signed provenance`,
  );
}

async function auditRegistryPackage({ name, version }) {
  const root = await mkdtemp(join(tmpdir(), "adocweave-textlint-npm-audit-"));
  const npm = npmInvocation();
  const environment = { ...process.env, npm_config_cache: join(root, ".npm-cache") };
  try {
    await writeFile(join(root, "package.json"), `${JSON.stringify({
      dependencies: { [name]: version },
      name: "adocweave-textlint-npm-audit",
      private: true,
    }, null, 2)}\n`);
    let result = await runProcess(npm.command, [
      ...npm.arguments,
      "install",
      "--ignore-scripts",
      "--legacy-peer-deps",
      "--no-audit",
      "--no-fund",
      "--prefer-online",
    ], { cwd: root, env: environment });
    if (result.code !== 0) {
      throw new Error(`npm install exited with ${result.code}\n${result.stdout}\n${result.stderr}`);
    }
    result = await runProcess(npm.command, [
      ...npm.arguments,
      "audit",
      "signatures",
      "--json",
      "--include-attestations",
    ], { cwd: root, env: environment });
    if (result.code !== 0) {
      throw new Error(
        `npm audit signatures exited with ${result.code}\n${result.stdout}\n${result.stderr}`,
      );
    }
    let report;
    try {
      report = JSON.parse(result.stdout);
    } catch (error) {
      throw new Error(`npm audit signatures returned invalid JSON: ${error.message}`);
    }
    assertSignatureAudit(report, { name, version });
  } finally {
    await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
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
    throw new Error(`npm Registry request failed with HTTP ${response.status}`);
  }
  return response.json();
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  const published = await runTextlintPluginNpmSmoke();
  process.stdout.write(
    `textlint plugin npm smoke passed: ${published.name}@${published.version}\n`
  );
}
