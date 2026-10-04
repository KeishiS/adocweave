import assert from "node:assert/strict";
import test from "node:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  cargoRuntimePackages,
  generateNativeThirdPartyNotices,
  renderCargoThirdPartyNotices,
  renderVscodeThirdPartyNotices,
  selectedThirdPartyPackages,
} from "./generate-third-party-notices.mjs";
import { vscodeRuntimePackages } from "./verify-vscode-dependencies.mjs";

const workspace = { id: "adocweave 1.2.3 (path+file:///workspace)", name: "adocweave", version: "1.2.3" };
const packageOf = (name, version, license) => ({ id: `${name} ${version} (registry+https://example.invalid)`, name, version, license });
const key = (name, version) => `${name}\0${version}`;

test("Cargo noticeには選択した配布runtime依存だけを含めます", () => {
  const adapter = { id: "adapter", name: "adocweave-textlint", version: "1.2.3" };
  const core = { id: "core", name: "adocweave", version: "1.2.3" };
  const alpha = packageOf("alpha", "1.0.0", "MIT");
  const beta = packageOf("beta", "2.0.0", "Apache-2.0");
  const metadata = {
    workspace_members: [adapter.id, core.id],
    packages: [adapter, core, alpha, beta],
  };
  const selected = new Set([key("alpha", "1.0.0")]);
  const packages = selectedThirdPartyPackages(metadata, selected);
  assert.deepEqual(packages, [
    { name: "alpha", version: "1.0.0", license: "MIT" },
  ]);
  const rendered = renderCargoThirdPartyNotices(packages, "textlint用ProcessorのNode.js向けWebAssembly");
  assert.match(rendered, /alpha 1\.0\.0/);
  assert.doesNotMatch(rendered, /beta 2\.0\.0/);
});

test("native noticeは埋め込むブラウザー資産の出典とlicense全文も含めます", () => {
  const temporary = mkdtempSync(join(tmpdir(), "adocweave-native-notices-"));
  try {
    const output = join(temporary, "THIRD_PARTY_NOTICES.adoc");
    generateNativeThirdPartyNotices("x86_64-unknown-linux-musl", output);
    const notice = readFileSync(output, "utf8");
    const directory = new URL("../crates/adocweave/assets/revealjs/", import.meta.url);
    const metadata = JSON.parse(readFileSync(new URL("sources.json", directory), "utf8"));
    for (const source of metadata.archives) {
      assert.ok(notice.includes(`${source.name} ${source.version}`));
      assert.ok(notice.includes(source.tarball));
      const asset = metadata.assets.find(asset => asset.archive === source.name && asset.path.startsWith("LICENSE."));
      assert.ok(notice.includes(readFileSync(new URL(asset.path, directory), "utf8").trimEnd()));
    }
    assert.doesNotMatch(notice, /@mathjax\/src|citeproc/);
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
});

test("選択した依存にSPDX license metadataがなければ拒否します", () => {
  const metadata = { workspace_members: [workspace.id], packages: [workspace, packageOf("missing", "1.0.0", null)] };
  assert.throws(
    () => selectedThirdPartyPackages(metadata, new Set([key("missing", "1.0.0")])),
    /missing 1\.0\.0 has no license metadata/,
  );
});

test("各Cargo配布物の依存集合は対象targetのnormal edgeと一致します", () => {
  const nativeNames = new Set(cargoRuntimePackages("adocweave", "x86_64-unknown-linux-musl").map(({ name }) => name));
  const wasmNames = new Set(cargoRuntimePackages("adocweave-wasm", "wasm32-unknown-unknown").map(({ name }) => name));
  const textlintNames = new Set(
    cargoRuntimePackages("adocweave-textlint", "wasm32-unknown-unknown").map(({ name }) => name),
  );
  assert.ok(nativeNames.has("clap"));
  assert.ok(!nativeNames.has("js-sys"));
  assert.ok(wasmNames.has("js-sys"));
  assert.ok(!wasmNames.has("clap"));
  assert.ok(textlintNames.has("serde-wasm-bindgen"));
  assert.ok(!textlintNames.has("clap"));
});

test("VS Code noticeには推移依存を含む配布runtime treeだけを含めます", () => {
  const packages = vscodeRuntimePackages(
    { private: true, version: "1.2.3" },
    {
      lockfileVersion: 3,
      packages: {
        "": { version: "1.2.3" },
        "node_modules/alpha": { version: "1.0.0", license: "MIT", resolved: "https://registry.npmjs.org/alpha" },
        "node_modules/alpha/node_modules/gamma": { version: "3.0.0", license: "ISC", resolved: "https://registry.npmjs.org/gamma" },
        "node_modules/beta": { dev: true, version: "2.0.0", license: "Apache-2.0", resolved: "https://registry.npmjs.org/beta" },
      },
    },
  );
  assert.deepEqual(packages.map(({ resolved: _resolved, ...pkg }) => pkg), [
    { name: "alpha", version: "1.0.0", license: "MIT" },
    { name: "gamma", version: "3.0.0", license: "ISC" },
  ]);
  const rendered = renderVscodeThirdPartyNotices(packages);
  assert.match(rendered, /alpha 1\.0\.0/);
  assert.match(rendered, /gamma 3\.0\.0/);
  assert.doesNotMatch(rendered, /beta 2\.0\.0/);
});
