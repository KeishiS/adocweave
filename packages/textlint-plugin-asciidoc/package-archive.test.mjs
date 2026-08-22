import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { gzipSync } from "node:zlib";

import {
  loadTextlintPluginManifest,
  TEXTLINT_PLUGIN_WASM_PATHS,
  validateTextlintPluginManifest,
} from "../../tools/textlint-plugin-package.mjs";
import { stageTextlintPluginPackage } from "../../tools/stage-textlint-plugin-package.mjs";
import { readTarMembers, verifyTextlintPluginPackage } from "../../tools/verify-textlint-plugin-package.mjs";

const manifest = loadTextlintPluginManifest();

test("公開manifestどおりのarchiveを検査する", async () => withArchive(entries(), async (archive) => {
  const result = await verifyTextlintPluginPackage(archive);
  assert.equal(result.fileCount, manifest.files.length + 1);
}));

test("package.jsonを正本としてstageを生成する", async () => {
  const root = await mkdtemp(join(tmpdir(), "adocweave-stage-test-"));
  const wasm = join(root, "wasm");
  const stage = join(root, "stage");
  const notice = join(root, "notice.adoc");
  try {
    await mkdir(wasm);
    await writeFile(join(wasm, "adocweave_textlint_wasm.js"), "module.exports = { parseText() {} };\n");
    await writeFile(join(wasm, "adocweave_textlint_wasm_bg.wasm"), minimalWasm());
    await writeFile(notice, "= Notice\n");
    const result = await stageTextlintPluginPackage(stage, wasm, notice);
    assert.deepEqual(result.manifest, manifest);
    assert.deepEqual(await listFiles(stage), ["package.json", ...manifest.files].sort());
    assert.deepEqual(JSON.parse(await readFile(join(stage, "package.json"), "utf8")), manifest);
    assert.equal(await readFile(join(stage, "THIRD_PARTY_NOTICES.adoc"), "utf8"), "= Notice\n");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("公開manifestのfile pathはportableな相対pathに限定する", () => {
  for (const path of [
    "/absolute",
    "../parent",
    "a/./b",
    "a//b",
    "a/",
    "C:/build",
    String.raw`C:\build`,
    String.raw`a\b`,
    "a\0b",
    "cafe\u0301",
  ]) {
    const mutant = structuredClone(manifest);
    mutant.files[0] = path;
    assert.throws(() => validateTextlintPluginManifest(mutant), /file pathが不正/);
  }
  const collision = structuredClone(manifest);
  collision.files[1] = collision.files[0].toUpperCase();
  assert.throws(() => validateTextlintPluginManifest(collision), /file pathが不正/);
});

test("欠落fileと余分なfileを拒否する", async () => {
  await withArchive(entries().slice(0, -1), (archive) => assert.rejects(verifyTextlintPluginPackage(archive), /file set/));
  await withArchive([...entries(), regular("package/extra.txt", "extra")], (archive) => assert.rejects(verifyTextlintPluginPackage(archive), /file set/));
});

test("symlink、hardlink、deviceを拒否する", async () => {
  for (const type of ["2", "1", "3"]) {
    const mutant = entries(); mutant[0] = { ...mutant[0], type };
    await withArchive(mutant, (archive) => assert.rejects(verifyTextlintPluginPackage(archive), /unsupported tar member type/));
  }
});

test("絶対path、親参照、backslash、drive、caseおよびUnicode衝突を拒否する", () => {
  for (const names of [
    ["/package/a"], ["package/../a"], [String.raw`package\a`], [String.raw`C:\package\a`],
    ["package/a", "package/A"], ["package/café", "package/cafe\u0301"],
  ]) assert.throws(() => readTarMembers(tarGz(names.map((name) => regular(name, "x")))), /path|collision/);
});

test("圧縮size budgetは境界を含み、1 byte超過を拒否する", async () => withArchive(entries(), async (archive, bytes) => {
  const unpacked = entries().reduce((sum, entry) => sum + entry.data.length, 0);
  await verifyTextlintPluginPackage(archive, { maximumPackedBytes: bytes.length, maximumUnpackedBytes: unpacked });
  await assert.rejects(verifyTextlintPluginPackage(archive, { maximumPackedBytes: bytes.length - 1 }), /packed size/);
  await assert.rejects(verifyTextlintPluginPackage(archive, { maximumUnpackedBytes: unpacked - 1 }), /unpacked size/);
}));

test("展開処理そのものを上限付きにする", () => {
  const bytes = gzipSync(Buffer.alloc(4096));
  assert.throws(
    () => readTarMembers(bytes, { maximumTarBytes: 4095 }),
    /cannot decompress archive within 4095 bytes/,
  );
});

test("WebAssemblyの機械固有pathを拒否する", async () => {
  const mutant = entries().map((entry) => entry.name.endsWith(".wasm")
    ? { ...entry, data: Buffer.concat([minimalWasm(), Buffer.from("/workspace/secret/source.rs")]) }
    : entry);
  await withArchive(mutant, (archive) => assert.rejects(verifyTextlintPluginPackage(archive), /machine-specific path/));

  const windows = entries().map((entry) => entry.name.endsWith(".wasm")
    ? { ...entry, data: Buffer.concat([minimalWasm(), Buffer.from("C:\\Users\\builder\\source.rs")]) }
    : entry);
  await withArchive(windows, (archive) => assert.rejects(verifyTextlintPluginPackage(archive), /machine-specific path/));
});

test("path形の偶然のbyte列は機械固有pathとして拒否しない", async () => {
  // 実際にCIで起きた誤検出の回帰test。custom section内の任意の3 byteが
  // drive文字のpatternへ一致してはいけません。
  const noiseSection = Buffer.from([0x00, 0x08, 0x01, 0x6e, 0x6f, 0x3a, 0x5c, 0x2f, 0x81, 0x00]);
  const noise = entries().map((entry) => entry.name.endsWith(".wasm")
    ? { ...entry, data: Buffer.concat([minimalWasm(), noiseSection]) }
    : entry);
  await withArchive(noise, (archive) => verifyTextlintPluginPackage(archive));
});

test("archive内manifestが正本と異なる場合は拒否する", async () => {
  const mutant = entries().map((entry) => entry.name === "package/package.json"
    ? { ...entry, data: Buffer.from(JSON.stringify({ ...JSON.parse(entry.data), sourceOnly: true })) }
    : entry);
  await withArchive(mutant, (archive) => assert.rejects(
    verifyTextlintPluginPackage(archive),
    /does not match the source package contract/,
  ));
});

function entries() {
  return ["package.json", ...manifest.files].map((path) => regular(`package/${path}`,
    path === "package.json" ? `${JSON.stringify(manifest)}\n`
      : path === TEXTLINT_PLUGIN_WASM_PATHS.wrapper
        ? "module.exports = { parseText() {} };\n"
        : path === TEXTLINT_PLUGIN_WASM_PATHS.binary ? minimalWasm() : sourceFor(path)));
}

import { readFileSync } from "node:fs";
function sourceFor(path) { try { return readFileSync(new URL(path === "LICENSE-APACHE" || path === "LICENSE-MIT" ? `../../${path}` : `./${path}`, import.meta.url)); } catch { return "fixture\n"; } }
function minimalWasm() { return Buffer.from([0,97,115,109,1,0,0,0,5,5,1,1,1,128,32]); }
function regular(name, data, type = "0") { return { data: Buffer.from(data), name, type }; }
async function withArchive(members, callback) { const root = await mkdtemp(join(tmpdir(), "adocweave-archive-test-")); const archive = join(root, "fixture.tgz"); const bytes = tarGz(members); try { await writeFile(archive, bytes); return await callback(archive, bytes); } finally { await rm(root, { recursive: true, force: true }); } }
function tarGz(members) { return gzipSync(Buffer.concat([...members.map(tarEntry), Buffer.alloc(1024)])); }
function tarEntry({ name, data, type = "0" }) {
  const header = Buffer.alloc(512); header.write(name, 0, 100, "utf8"); writeOctal(header, 100, 8, 0o644); writeOctal(header, 108, 8, 0); writeOctal(header, 116, 8, 0); writeOctal(header, 124, 12, data.length); writeOctal(header, 136, 12, 0); header.fill(0x20, 148, 156); header.write(type, 156, 1, "ascii"); header.write("ustar\0", 257, 6, "ascii"); header.write("00", 263, 2, "ascii"); writeOctal(header, 148, 8, [...header].reduce((sum, byte) => sum + byte, 0)); return Buffer.concat([header, data, Buffer.alloc((512 - data.length % 512) % 512)]);
}
function writeOctal(buffer, offset, length, value) { buffer.write(`${value.toString(8).padStart(length - 1, "0")}\0`, offset, length, "ascii"); }

async function listFiles(root, prefix = "") {
  const result = [];
  for (const entry of await readdir(join(root, prefix), { withFileTypes: true })) {
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) result.push(...await listFiles(root, relative));
    else result.push(relative);
  }
  return result.sort();
}
