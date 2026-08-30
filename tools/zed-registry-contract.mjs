import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const ROOT = new URL("../", import.meta.url);
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
assert.equal(manifestValue(manifest, "id"), "adocweave");
assert.equal(manifestValue(manifest, "name"), "AdocWeave");
assert.equal(manifestValue(manifest, "repository"), "https://github.com/KeishiS/adocweave");
assert.match(manifest, /^schema_version = 1$/mu);
assert.match(manifest, /^\[language_servers\.adocweave\]$/mu);
assert.match(manifest, /^languages = \["AsciiDoc"\]$/mu);
assert.doesNotMatch(manifest, /^\[grammars\./mu);

const cargoManifest = read(new URL("Cargo.toml", ZED));
assert.match(cargoManifest, /^version = "0\.0\.0"$/mu);
assert.doesNotMatch(cargoManifest, new RegExp(`^version = "${version.replaceAll(".", "\\.")}"$`, "mu"));
const toolchains = JSON.parse(read(new URL("toolchains.json", ROOT)));
const rustVersion = /^rust-version = "([^"]+)"$/mu.exec(cargoManifest);
assert.ok(rustVersion, "editors/zed/Cargo.toml is missing rust-version");
assert.equal(rustVersion[1], toolchains.rustVersion, "Zed Rust version does not match toolchains.json");

const license = read(new URL("LICENSE", ZED));
assert.match(license, /^MIT License\n/u);

const readme = read(new URL("README.md", ZED));
assert.match(readme, /<img src="icon\.png" width="128" height="128" alt="AdocWeave icon">/u);
assert.match(readme, /AdocWeave 0\.51\.0 or newer is required/u);
assert.match(readme, /Install Zed's `AsciiDoc` extension first/u);

const changelog = read(new URL("CHANGELOG.md", ZED));
assert.match(changelog, new RegExp(`^## \\[${version.replaceAll(".", "\\.")}\\]$`, "mu"));

const icon = read(new URL("icon.png", ZED), null);
assert.deepEqual([...icon.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
assert.equal(icon.readUInt32BE(16), 128);
assert.equal(icon.readUInt32BE(20), 128);

for (const path of ["README.md", "CHANGELOG.md", "src/acquire.rs", "src/lib.rs"]) {
  assert.doesNotMatch(read(new URL(path, ZED)), /[ぁ-んァ-ヶ一-龠]/u, `${path} contains non-English text`);
}

const extensionSource = read(new URL("src/lib.rs", ZED));
assert.match(extensionSource, /latest_github_release/u);
assert.match(extensionSource, /pre_release: false/u);
assert.doesNotMatch(extensionSource, /latest_lsp_release|RELEASES_URL/u);

const versionSync = read(new URL("release/version-sync.json", ROOT));
assert.doesNotMatch(versionSync, /editors\/zed/u);
assert.doesNotMatch(read(new URL("dist-workspace.toml", ROOT)), /adocweave-zed/u);
assert.doesNotMatch(read(new URL("Makefile.toml", ROOT)), /package-zed-release|test-zed-release/u);

const developmentGuide = read(new URL("docs/developer-guide/zed-development.adoc", ROOT));
for (const required of [
  "[adocweave]",
  'submodule = "extensions/adocweave"',
  'path = "editors/zed"',
  'version = "<extension.tomlのversion>"',
]) {
  assert.ok(developmentGuide.includes(required), `Zed registry submission example is missing: ${required}`);
}
assert.match(developmentGuide, /Zed公式レジストリの\n``AsciiDoc``拡張/u);

process.stdout.write(`Zed registry contract verified: ${version}\n`);
