import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import vm from "node:vm";

const defaultDirectory = fileURLToPath(new URL("../crates/adocweave/assets/revealjs/", import.meta.url));
const hash = (bytes, encoding = "hex", algorithm = "sha256") => createHash(algorithm).update(bytes).digest(encoding);

export function verifySlidesAssets(directory = defaultDirectory, archiveDirectory) {
  const metadata = JSON.parse(readFileSync(resolve(directory, "sources.json"), "utf8"));
  assert.equal(metadata.schemaVersion, 1);
  assert.deepEqual(metadata.archives.map(source => source.name), ["reveal.js", "fitty", "marked"]);
  assert.deepEqual(metadata.assets.map(asset => asset.path).sort(), [
    "LICENSE.fitty.txt", "LICENSE.marked.txt", "LICENSE.revealjs.txt",
    "notes.js", "reset.css", "reveal.css", "reveal.js"
  ]);
  assert.deepEqual(readdirSync(directory).sort(), metadata.assets.map(asset => asset.path).concat(["NOTICE.txt", "sources.json"]).sort());
  for (const source of metadata.archives) {
    assert.equal(source.tarball, `https://registry.npmjs.org/${source.name}/-/${source.name}-${source.version}.tgz`);
    assert.match(source.sha256, /^[a-f0-9]{64}$/);
    assert.match(source.integrity, /^sha512-[A-Za-z0-9+/]+={0,2}$/);
    if (archiveDirectory) {
      const archive = readFileSync(resolve(archiveDirectory, `${source.name}-${source.version}.tgz`));
      assert.equal(hash(archive), source.sha256, `archive digest: ${source.name}`);
      assert.equal(`sha512-${hash(archive, "base64", "sha512")}`, source.integrity, `archive integrity: ${source.name}`);
    }
  }
  for (const asset of metadata.assets) {
    assert.equal(hash(readFileSync(resolve(directory, asset.path))), asset.sha256, `asset digest: ${asset.path}`);
  }
  assert.match(readFileSync(resolve(directory, "reveal.js"), "utf8"), /typeof exports/);
  const notice = readFileSync(resolve(directory, "NOTICE.txt"), "utf8");
  for (const source of metadata.archives) assert.ok(notice.includes(`${source.name} ${source.version}`));

  // Execute only the pinned notes plugin in an isolated context with no I/O.
  let speaker;
  const context = {
    window: {
      location: { search: "", protocol: "file:", host: "", pathname: "/index.html" },
      addEventListener() {},
      open() { return { document: { write(html) { speaker = html; } }, closed: false }; }
    },
    setInterval() { return 1; }, clearInterval() {}, alert() {}
  };
  vm.runInNewContext(readFileSync(resolve(directory, "notes.js"), "utf8"), context, { timeout: 1000 });
  const notes = typeof context.RevealNotes === "function" ? context.RevealNotes() : context.RevealNotes;
  notes.init({ getConfig() { return {}; }, getState() { return {}; }, addKeyBinding() {}, on() {} });
  notes.open();
  assert.equal(typeof speaker, "string");
  assert.ok(!/\son\w+=/i.test(speaker), "speaker view has no inline event handlers");
  const scripts = [...speaker.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/gi)]
    .map(match => ({ sha256: hash(match[1], "base64"), sizeBytes: Buffer.byteLength(match[1]) }));
  assert.deepEqual(scripts, metadata.speakerScripts, "speaker inline scripts must match the CSP hashes");
  return metadata;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  verifySlidesAssets(defaultDirectory, process.argv[2]);
  console.log("Pinned reveal.js assets, licenses and speaker CSP hashes verified.");
}
