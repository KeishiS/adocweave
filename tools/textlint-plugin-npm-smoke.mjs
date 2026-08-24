import process from "node:process";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

import { runTextlintPluginConsumerE2E } from "./textlint-plugin-consumer-e2e.mjs";
import { runTextlintPluginNpxSmoke } from "./textlint-plugin-npx-smoke.mjs";
import { loadTextlintPluginManifest } from "./textlint-plugin-package.mjs";

// 公開したpackageをnpm Registryから導入し直して観測する。GitHub Releaseの
// archiveを直接指す既存のsmokeとは別に、registryのversion解決を経る導入だけを扱う。
export async function runTextlintPluginNpmSmoke({
  manifest = loadTextlintPluginManifest(),
  fetchJson = defaultFetchJson,
  runConsumerE2E = runTextlintPluginConsumerE2E,
  runNpxSmoke = runTextlintPluginNpxSmoke
} = {}) {
  const { name, version } = manifest;
  if (typeof version !== "string") {
    throw new Error("textlint packageのversionを解釈できません");
  }
  const metadata = await fetchJson(`https://registry.npmjs.org/${name}/${version}`);
  if (metadata?.name !== name || metadata?.version !== version) {
    throw new Error(
      `npmに${name}@${version}が見つかりません。公開後smokeは公開のあとに実行してください。`
    );
  }
  const spec = `${name}@${version}`;
  await runConsumerE2E(spec, { manifest });
  await runNpxSmoke(spec, { manifest });
  return Object.freeze({ name, version });
}

async function defaultFetchJson(url) {
  const response = await fetch(url, { headers: { accept: "application/json" } });
  if (!response.ok) {
    throw new Error(`npm registryへの問い合わせに失敗しました: HTTP ${response.status}`);
  }
  return response.json();
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  const published = await runTextlintPluginNpmSmoke();
  process.stdout.write(
    `textlint plugin npm smoke passed: ${published.name}@${published.version}\n`
  );
}
