import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const ZED = new URL("../editors/zed/", import.meta.url);

function read(url, encoding = "utf8") {
  return readFileSync(url, encoding);
}

function manifestValue(source, key) {
  const match = new RegExp(`^${key} = "([^"]+)"$`, "mu").exec(source);
  assert.ok(match, `extension.toml is missing ${key}`);
  return match[1];
}

const manifest = read(new URL("extension.toml", ZED));
const version = manifestValue(manifest, "version");
assert.match(version, /^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)$/u);
assert.equal(manifestValue(manifest, "id"), "adocweave-lsp");
assert.equal(manifestValue(manifest, "name"), "AdocWeave");
assert.equal(manifestValue(manifest, "repository"), "https://github.com/KeishiS/adocweave");
assert.match(manifest, /^schema_version = 1$/mu);
assert.match(manifest, /^\[language_servers\.adocweave\]$/mu);
assert.match(manifest, /^languages = \["AsciiDoc"\]$/mu);
assert.doesNotMatch(manifest, /^\[grammars\./mu);

const cargoManifest = read(new URL("Cargo.toml", ZED));
assert.match(cargoManifest, /^version = "0\.0\.0"$/mu);

const license = read(new URL("LICENSE", ZED));
assert.match(license, /^MIT License\n/u);

const changelog = read(new URL("CHANGELOG.md", ZED));
assert.match(changelog, new RegExp(`^## \\[${version.replaceAll(".", "\\.")}\\]$`, "mu"));

const icon = read(new URL("icon.png", ZED), null);
assert.deepEqual([...icon.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
assert.equal(icon.readUInt32BE(16), 128);
assert.equal(icon.readUInt32BE(20), 128);

for (const path of ["README.md", "CHANGELOG.md", "src/acquire.rs", "src/lib.rs"]) {
  assert.doesNotMatch(read(new URL(path, ZED)), /[ぁ-んァ-ヶ一-龠]/u, `${path} contains non-English text`);
}

process.stdout.write(`Zed registry requirements verified: ${version}\n`);
