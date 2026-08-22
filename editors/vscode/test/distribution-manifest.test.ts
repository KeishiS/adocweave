import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { parseDistributionManifest, selectLspAsset } from "../src/distribution-manifest.js";
import { platformForHost } from "../src/platform.js";

const legacyFixture = JSON.parse(
  readFileSync("../../release/adocweave-dist-manifest.fixture.json", "utf8"),
);
const fixtureVersion = legacyFixture.productVersion ?? legacyFixture.packageVersion;
const { packageVersion: _packageVersion, ...legacyFields } = legacyFixture;
const fixtureValue = {
  ...legacyFields,
  assets: legacyFields.assets.filter(({ kind }: { kind: string }) => kind === "lsp"),
  lspApiVersion: 1,
  product: "adocweave-lsp",
  productVersion: fixtureVersion,
  schemaVersion: 3,
};
const fixture = JSON.stringify(fixtureValue);

test("公開manifestからplatformに一致するassetを一意に選択します", () => {
  const manifest = parseDistributionManifest(fixture, fixtureVersion, [1]);
  const asset = selectLspAsset(manifest, platformForHost("win32", "x64", "10.0.17763"));
  assert.equal(asset.name, "adocweave-lsp-x86_64-pc-windows-msvc.zip");
  assert.equal(asset.executable, "adocweave-lsp.exe");
});

test("未知field、version不一致、重複assetを拒否します", () => {
  const parsed = fixtureValue;
  assert.throws(
    () =>
      parseDistributionManifest(
        JSON.stringify({
          assets: parsed.assets,
          packageVersion: fixtureVersion,
          schemaVersion: 2,
          sourceCommit: parsed.sourceCommit,
        }),
        fixtureVersion,
        [1],
      ),
    /invalid-manifest/,
  );
  assert.throws(
    () =>
      parseDistributionManifest(JSON.stringify({ ...parsed, unknown: true }), fixtureVersion, [1]),
    /invalid-manifest/,
  );
  assert.throws(() => parseDistributionManifest(fixture, "9.9.9", [1]), /invalid-manifest/);
  assert.throws(() => parseDistributionManifest(fixture, fixtureVersion, [2]), /invalid-manifest/);
  const windowsLsp = parsed.assets.find(
    ({ name }: { name: string }) => name === "adocweave-lsp-x86_64-pc-windows-msvc.zip",
  );
  const manifest = parseDistributionManifest(
    JSON.stringify({ ...parsed, assets: [...parsed.assets, windowsLsp] }),
    fixtureVersion,
    [1],
  );
  assert.throws(
    () => selectLspAsset(manifest, platformForHost("win32", "x64", "10.0.17763")),
    /asset-count/,
  );
});
