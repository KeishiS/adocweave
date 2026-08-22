import { createRequire } from "node:module";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import process from "node:process";
import { gunzipSync } from "node:zlib";
import { pathToFileURL } from "node:url";

import {
  loadTextlintPluginManifest,
  TEXTLINT_PLUGIN_PACKAGE_LIMITS,
  TEXTLINT_PLUGIN_WASM_PATHS,
  validateTextlintPluginManifest,
} from "./textlint-plugin-package.mjs";
import { verifyMemoryMaximum } from "./verify-textlint-wasm-memory.mjs";

function fail(message) { throw new Error(`textlint plugin archive: ${message}`); }
function octal(bytes) {
  const text = bytes.toString("ascii").replace(/\0.*$/, "").trim();
  if (!/^[0-7]*$/.test(text)) fail("tar header has an invalid octal field");
  return text === "" ? 0 : Number.parseInt(text, 8);
}
function canonicalArchivePath(name) {
  if (name.includes("\\") || name.startsWith("/") || /^[A-Za-z]:/.test(name)) fail(`unsafe archive path: ${name}`);
  const parts = name.split("/");
  if (parts.some((part) => part === "" || part === "." || part === "..")) fail(`non-canonical archive path: ${name}`);
  if (name.normalize("NFC") !== name) fail(`non-canonical Unicode archive path: ${name}`);
  return name;
}

export function readTarMembers(tgzBytes, { maximumTarBytes = 64 * 1024 * 1024 } = {}) {
  let tar;
  try {
    tar = gunzipSync(tgzBytes, { maxOutputLength: maximumTarBytes });
  } catch (error) {
    fail(`cannot decompress archive within ${maximumTarBytes} bytes: ${error.message}`);
  }
  if (tar.length % 512 !== 0) fail("tar length is not block-aligned");
  const members = [];
  const collisionKeys = new Set();
  let ended = false;
  for (let offset = 0; offset + 512 <= tar.length;) {
    const header = tar.subarray(offset, offset + 512); offset += 512;
    if (header.every((byte) => byte === 0)) {
      if (!tar.subarray(offset).every((byte) => byte === 0)) fail("tar contains data after its end marker");
      ended = true;
      break;
    }
    const recordedChecksum = octal(header.subarray(148, 156));
    const checksumHeader = Buffer.from(header); checksumHeader.fill(0x20, 148, 156);
    if (recordedChecksum !== checksumHeader.reduce((sum, byte) => sum + byte, 0)) fail("tar header checksum does not match");
    const namePart = header.subarray(0, 100).toString("utf8").replace(/\0.*$/, "");
    const prefix = header.subarray(345, 500).toString("utf8").replace(/\0.*$/, "");
    const name = canonicalArchivePath(prefix ? `${prefix}/${namePart}` : namePart);
    const size = octal(header.subarray(124, 136));
    const type = header[156] === 0 ? "0" : String.fromCharCode(header[156]);
    if (type !== "0") fail(`unsupported tar member type ${JSON.stringify(type)}: ${name}`);
    if (!Number.isSafeInteger(size) || offset + size > tar.length) fail(`truncated tar member: ${name}`);
    const key = name.normalize("NFC").toLocaleLowerCase("en-US");
    if (collisionKeys.has(key)) fail(`duplicate or portable-name collision: ${name}`);
    collisionKeys.add(key);
    members.push({ data: Buffer.from(tar.subarray(offset, offset + size)), name, size, type });
    offset += Math.ceil(size / 512) * 512;
  }
  if (!ended) fail("tar has no end marker");
  return members;
}

// Both alternatives require a known machine-directory name, not just a path
// shape: the WebAssembly is binary, and three arbitrary bytes such as "o:\"
// otherwise satisfy a bare drive-letter pattern.
function assertNoMachinePath(bytes) {
  const text = bytes.toString("latin1");
  const match = text.match(/(?:^|[^A-Za-z0-9_])(\/(?:workspace|home|Users|tmp|private\/tmp|builds?|runner|__w)\/[!-~]{0,120})/) ??
    text.match(/([A-Za-z]:\\(?:Users|Windows|a|w|temp|tmp|hostedtoolcache|actions-runner)\\[!-~]{0,120})/i);
  if (match) fail(`WebAssembly contains a machine-specific path: ${match[1]}`);
}

export async function verifyTextlintPluginPackage(archivePath, { maximumPackedBytes, maximumUnpackedBytes } = {}) {
  const sourceManifest = loadTextlintPluginManifest();
  const archive = resolve(archivePath);
  const packed = await readFile(archive);
  const packedLimit = maximumPackedBytes ?? TEXTLINT_PLUGIN_PACKAGE_LIMITS.maximumPackedBytes;
  if (packed.length > packedLimit) fail(`packed size exceeds ${packedLimit} bytes`);
  const maximumTarBytes = maximumUnpackedBytes ?? TEXTLINT_PLUGIN_PACKAGE_LIMITS.maximumUnpackedBytes;
  const tarOverhead = (sourceManifest.files.length + 1) * (512 + 511) + 1024;
  const members = readTarMembers(packed, { maximumTarBytes: maximumTarBytes + tarOverhead });
  const byName = new Map(members.map((member) => [member.name, member.data]));
  const manifestBytes = byName.get("package/package.json");
  if (!manifestBytes) fail("package.json is missing");
  const manifest = JSON.parse(manifestBytes.toString("utf8"));
  validateTextlintPluginManifest(manifest);
  if (JSON.stringify(manifest) !== JSON.stringify(sourceManifest)) {
    fail("archive package.json does not match the source package contract");
  }
  const expectedPaths = ["package.json", ...manifest.files];
  const expected = expectedPaths.map((path) => `package/${path}`).sort();
  const actual = members.map(({ name }) => name).sort();
  if (members.length !== expectedPaths.length || JSON.stringify(actual) !== JSON.stringify(expected)) {
    fail("file set does not match package.json");
  }
  const unpacked = members.reduce((sum, member) => sum + member.size, 0);
  const unpackedLimit = maximumUnpackedBytes ?? TEXTLINT_PLUGIN_PACKAGE_LIMITS.maximumUnpackedBytes;
  if (unpacked > unpackedLimit) fail(`unpacked size exceeds ${unpackedLimit} bytes`);
  const packageFiles = new Map(members.map((member) => [member.name.slice("package/".length), member.data]));
  for (const field of ["dependencies", "devDependencies", "optionalDependencies", "bundledDependencies"]) {
    const value = manifest[field];
    if (value && (Array.isArray(value) ? value.length : Object.keys(value).length) !== 0) fail(`package must not contain ${field}`);
  }
  if (manifest.scripts && Object.keys(manifest.scripts).length !== 0) fail("package must not define scripts");
  const wasm = packageFiles.get(TEXTLINT_PLUGIN_WASM_PATHS.binary);
  assertNoMachinePath(wasm);
  verifyMemoryMaximum(wasm, TEXTLINT_PLUGIN_PACKAGE_LIMITS.maximumMemoryBytes);
  const root = await mkdtemp(join(tmpdir(), "adocweave-textlint-archive-"));
  try {
    for (const member of members) { const relative = member.name.slice("package/".length); const target = join(root, relative); await mkdir(dirname(target), { recursive: true }); await writeFile(target, member.data, { flag: "wx" }); }
    const wrapper = createRequire(import.meta.url)(join(root, TEXTLINT_PLUGIN_WASM_PATHS.wrapper));
    if (JSON.stringify(Object.keys(wrapper)) !== JSON.stringify(["parseText"])) {
      fail("WebAssembly wrapper must export parseText only");
    }
  } finally { await rm(root, { recursive: true, force: true }); }
  return { fileCount: members.length, packedBytes: packed.length, unpackedBytes: unpacked };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [archive] = process.argv.slice(2);
  if (!archive) { process.stderr.write("usage: node tools/verify-textlint-plugin-package.mjs PACKAGE_TGZ\n"); process.exit(2); }
  try { const result = await verifyTextlintPluginPackage(archive); process.stdout.write(`textlint plugin archive verified: ${result.fileCount} files\n`); }
  catch (error) { process.stderr.write(`${error.message}\n`); process.exitCode = 1; }
}
