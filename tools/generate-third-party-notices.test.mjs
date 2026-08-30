import assert from "node:assert/strict";
import test from "node:test";

import {
  cargoTreePackageKeys,
  reachableThirdPartyPackages,
  renderTextlintPluginNotices,
  renderThirdPartyNotices,
  renderVscodeThirdPartyNotices,
  thirdPartyPackages,
} from "./generate-third-party-notices.mjs";
import { vscodeRuntimePackages } from "./verify-vscode-dependencies.mjs";

const workspace = { id: "adocweave 1.2.3 (path+file:///workspace)", name: "adocweave", version: "1.2.3" };
const packageOf = (name, version, license) => ({ id: `${name} ${version} (registry+https://example.invalid)`, name, version, license });

test("notice rendering groups native and VS Code runtime dependencies", () => {
  const root = {
    workspace_members: [workspace.id],
    packages: [workspace, packageOf("alpha", "1.0.0", "MIT"), packageOf("beta", "2.0.0", "Apache-2.0")],
  };
  const rendered = renderThirdPartyNotices(root, [
    { name: "delta", version: "4.0.0", license: "MIT" },
  ]);
  assert.match(rendered, /\|Apache-2\.0\n\|beta 2\.0\.0/);
  assert.match(rendered, /\|MIT\n\|alpha 1\.0\.0/);
  assert.doesNotMatch(rendered, /Zed開発拡張archive/u);
  assert.match(rendered, /== VS Code拡張の実行時依存[\s\S]*\|MIT\n\|delta 4\.0\.0/);
});

test("textlint plugin noticeには専用WASMから到達する依存だけを含めます", () => {
  const adapter = { id: "adapter", name: "adocweave-textlint", version: "1.2.3" };
  const core = { id: "core", name: "adocweave", version: "1.2.3" };
  const alpha = packageOf("alpha", "1.0.0", "MIT");
  const beta = packageOf("beta", "2.0.0", "Apache-2.0");
  const metadata = {
    workspace_members: [adapter.id, core.id],
    packages: [adapter, core, alpha, beta],
    resolve: {
      nodes: [
        { id: adapter.id, deps: [{ pkg: core.id }] },
        { id: core.id, deps: [{ pkg: alpha.id }] },
        { id: alpha.id, deps: [] },
        { id: beta.id, deps: [] },
      ],
    },
  };
  assert.deepEqual(reachableThirdPartyPackages(metadata, adapter.name), [
    { name: "alpha", version: "1.0.0", license: "MIT" },
  ]);
  const rendered = renderTextlintPluginNotices(metadata);
  assert.match(rendered, /alpha 1\.0\.0/);
  assert.doesNotMatch(rendered, /beta 2\.0\.0/);
});

test("notice rendering rejects dependencies without SPDX license metadata", () => {
  const metadata = { workspace_members: [workspace.id], packages: [workspace, packageOf("missing", "1.0.0", null)] };
  assert.throws(() => thirdPartyPackages(metadata), /missing 1\.0\.0 has no license metadata/);
});

test("textlint pluginの依存集合はwasm32向けnormal edgeと一致します", () => {
  const key = (name, version) => `${name}\0${version}`;
  const packages = cargoTreePackageKeys(
    "adocweave-textlint",
    "wasm32-unknown-unknown",
  );
  assert.ok([...packages].some((key) => key.startsWith("adocweave-textlint\0")));
  assert.ok(packages.has(key("serde-wasm-bindgen", "0.6.5")));
  assert.ok(!packages.has(key("futures-channel", "0.3.33")));
  assert.ok(!packages.has(key("const-oid", "0.10.2")));
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
