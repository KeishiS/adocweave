import { cp, mkdir, rm } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  loadTextlintPluginManifest,
  TEXTLINT_PLUGIN_WASM_PATHS,
} from "./textlint-plugin-package.mjs";

const ROOT = fileURLToPath(new URL("../", import.meta.url));

export async function stageTextlintPluginPackage(stageDirectory, wasmDirectory, noticeFile) {
  const manifest = loadTextlintPluginManifest();
  const stage = resolve(stageDirectory);
  await rm(stage, { recursive: true, force: true });
  await mkdir(stage, { recursive: true });
  await cp(
    join(ROOT, "packages/textlint-plugin-asciidoc/package.json"),
    join(stage, "package.json"),
    { errorOnExist: true },
  );
  for (const path of manifest.files) {
    const destination = join(stage, path);
    await mkdir(dirname(destination), { recursive: true });
    if (path === "THIRD_PARTY_NOTICES.adoc") {
      await cp(resolve(noticeFile), destination, { errorOnExist: true });
    } else if (path === TEXTLINT_PLUGIN_WASM_PATHS.wrapper) {
      await cp(join(resolve(wasmDirectory), "adocweave_textlint_wasm.js"), destination, { errorOnExist: true });
    } else if (path === TEXTLINT_PLUGIN_WASM_PATHS.binary) {
      await cp(join(resolve(wasmDirectory), "adocweave_textlint_wasm_bg.wasm"), destination, { errorOnExist: true });
    } else if (path === "LICENSE-APACHE" || path === "LICENSE-MIT") {
      await cp(join(ROOT, path), destination, { errorOnExist: true });
    } else {
      await cp(join(ROOT, "packages/textlint-plugin-asciidoc", path), destination, { errorOnExist: true });
    }
  }
  return { manifest, stage };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [stage, wasm, notice] = process.argv.slice(2);
  if (!stage || !wasm || !notice) {
    process.stderr.write("usage: node tools/stage-textlint-plugin-package.mjs STAGE_DIR WASM_DIR NOTICE_FILE\n");
    process.exit(2);
  }
  try { const result = await stageTextlintPluginPackage(stage, wasm, notice); process.stdout.write(`textlint plugin package staged: ${result.stage}\n`); }
  catch (error) { process.stderr.write(`${error.message}\n`); process.exitCode = 1; }
}
