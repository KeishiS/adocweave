import assert from "node:assert/strict";
import test from "node:test";

import {
  EXPECTED_RELEASE_METADATA,
  canonicalJson,
  expectedAssets,
  productVersions,
  releaseFromTag,
  validateDistributionManifest,
  validateDistPlan,
  validateReleaseIdentity,
} from "./release-contract.mjs";
import plan from "../release/distribution-plan.json" with { type: "json" };
import fixture from "../release/adocweave-dist-manifest.fixture.json" with { type: "json" };

test("stable tags identify one product and one version", () => {
  for (const product of ["cli", "lsp", "browser", "textlint", "vscode", "zed"]) {
    const tag = `adocweave-${product}/v1.2.3`;
    assert.deepEqual(releaseFromTag(tag), { product, productVersion: "1.2.3" });
    assert.doesNotThrow(() => validateReleaseIdentity(product, "1.2.3", tag, plan));
  }
  for (const invalid of [
    "v1.2.3",
    "adocweave/v1.2.3",
    "adocweave-lsp/1.2.3",
    "adocweave-lsp/v1.2",
    "adocweave-lsp/v1.2.3-rc.1",
  ]) {
    assert.throws(() => releaseFromTag(invalid));
  }
  assert.throws(
    () => validateReleaseIdentity("cli", "1.2.3", "adocweave-lsp/v1.2.3", plan),
    /does not identify/,
  );
});

test("distribution plan routes six products without storing their versions", () => {
  assert.equal(plan.schemaVersion, 3);
  assert.equal("packageVersion" in plan, false);
  assert.equal("assets" in plan, false);
  assert.deepEqual(plan.products.map(({ product }) => product), [
    "cli",
    "lsp",
    "browser",
    "textlint",
    "vscode",
    "zed",
  ]);
  const routes = Object.fromEntries(plan.products.map((route) => [route.product, route]));
  assert.equal(routes.cli.versionSource, "crates/adocweave-cli/Cargo.toml#package.version");
  assert.equal(routes.lsp.versionSource, "crates/adocweave-lsp/Cargo.toml#package.version");
  assert.equal(routes.textlint.versionSource, "packages/textlint-plugin-asciidoc/package.json#version");
  const versions = productVersions(plan);
  assert.deepEqual(Object.keys(versions), plan.products.map(({ product }) => product));
  assert.ok(Object.values(versions).every((version) => /^\d+\.\d+\.\d+$/.test(version)));
  assert.deepEqual(plan.releaseMetadata, EXPECTED_RELEASE_METADATA);
});

test("expected assets contain only the selected product", () => {
  const cli = expectedAssets("cli", "1.2.3", plan.targets);
  assert.equal(cli.length, plan.targets.length);
  assert.ok(cli.every(({ kind, name }) => kind === "cli" && name.startsWith("adocweave-cli-")));

  const lsp = expectedAssets("lsp", "2.3.4", plan.targets);
  assert.equal(lsp.length, plan.targets.length);
  assert.ok(lsp.every(({ kind, name }) => kind === "lsp" && name.startsWith("adocweave-lsp-")));

  assert.deepEqual(expectedAssets("browser", "3.4.5", plan.targets), [{
    name: "adocweave-browser-3.4.5.tgz",
    kind: "browser",
    target: null,
    archive: "tgz",
    executable: null,
  }]);
  assert.deepEqual(expectedAssets("textlint", "4.5.6", plan.targets), [{
    name: "adocweave-textlint-plugin-asciidoc-4.5.6.tgz",
    kind: "textlint-plugin",
    target: null,
    archive: "tgz",
    executable: null,
  }]);
  assert.deepEqual(expectedAssets("vscode", "5.6.7", plan.targets), [{
    name: "adocweave-vscode-5.6.7.vsix",
    kind: "vscode",
    target: null,
    archive: "vsix",
    executable: null,
  }]);
  assert.deepEqual(expectedAssets("zed", "6.7.8", plan.targets), [{
    name: "adocweave-zed-6.7.8.tar.xz",
    kind: "zed",
    target: null,
    archive: "tar.xz",
    executable: null,
  }]);
});

test("distribution manifest fixture satisfies the LSP product contract", () => {
  assert.doesNotThrow(() => validateDistributionManifest(fixture, plan));
  assert.equal(fixture.product, "lsp");
  assert.equal(fixture.schemaVersion, 5);
  assert.equal("lspApiVersion" in fixture, false);
  assert.ok(fixture.assets.every((asset) => !("kind" in asset)));
  assert.equal(canonicalJson(fixture), `${JSON.stringify(fixture, null, 2)}\n`);
});

test("manifest rejects another product asset and unknown fields", () => {
  assert.throws(() => validateDistributionManifest({ ...fixture, unexpected: true }, plan));
  assert.throws(() => validateDistributionManifest({
    ...fixture,
    assets: [fixture.assets[1], fixture.assets[0], ...fixture.assets.slice(2)],
  }, plan));
  assert.throws(() => validateDistributionManifest({ ...fixture, lspApiVersion: 1 }, plan));
  assert.throws(() => validateDistributionManifest({
    ...fixture,
    assets: fixture.assets.map((asset, index) => index === 0
      ? { ...asset, kind: "cli" }
      : asset),
  }, plan));
});

function cargoDistPlan(product, version) {
  const route = plan.products.find((entry) => entry.product === product);
  const assets = expectedAssets(product, version, plan.targets);
  return {
    dist_version: plan.distVersion,
    announcement_tag: `${route.tagPrefix}${version}`,
    releases: [{ app_name: route.package, app_version: version }],
    artifacts: Object.fromEntries(assets.map((asset) => [asset.name, {
      name: asset.name,
      kind: "executable-zip",
      target_triples: [asset.target],
      assets: [
        { kind: "executable", path: asset.executable },
        ...["LICENSE-APACHE", "LICENSE-MIT", "README.adoc", "THIRD_PARTY_NOTICES.adoc"]
          .map((name) => ({ kind: "misc", name })),
      ],
    }])),
    ci: {
      github: {
        artifacts_matrix: {
          include: plan.targets.map((target) => ({ targets: [target.triple], runner: target.runner })),
        },
      },
    },
  };
}

test("cargo-dist singular tags announce either CLI or LSP, never both", () => {
  for (const product of ["cli", "lsp"]) {
    const version = "1.2.3";
    const tag = `adocweave-${product}/v${version}`;
    const distPlan = cargoDistPlan(product, version);
    assert.doesNotThrow(() => validateDistPlan(distPlan, plan, product, version, tag));
    const other = product === "cli" ? "adocweave-lsp" : "adocweave-cli";
    assert.throws(() => validateDistPlan({
      ...distPlan,
      releases: [...distPlan.releases, { app_name: other, app_version: version }],
    }, plan, product, version, tag), /exactly/);
  }
  assert.throws(() => validateDistPlan(
    cargoDistPlan("cli", "1.2.3"),
    plan,
    "lsp",
    "1.2.3",
    "adocweave-lsp/v1.2.3",
  ));
});
