import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  createPublicationPlan,
  loadDistributionPlan,
  productAssets,
  productIdentity,
  productTag,
  productVersion,
  readVersionSource,
  selectProduct,
  validateProductCandidate,
  validateProductPublication,
  validatePublicationPlan,
} from "./product-release.mjs";

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "adocweave-product-release-"));
  for (const directory of ["release", "crates/tool", "web"]) mkdirSync(join(root, directory), { recursive: true });
  writeFileSync(join(root, "crates/tool/Cargo.toml"), '[package]\nname = "tool"\nversion = "1.2.3"\n');
  writeFileSync(join(root, "web/package.json"), '{"name":"web","version":"4.5.6"}\n');
  const plan = {
    schemaVersion: 3,
    products: [
      {
        product: "cli",
        versionSource: "crates/tool/Cargo.toml#package.version",
        tagPrefix: "adocweave-cli/v",
        assetKind: "cli",
        assetName: "adocweave-cli-{target}.zip",
        archive: "zip",
        executable: "adocweave{executableSuffix}",
        build: "cargo-dist",
        package: "tool",
      },
      {
        product: "lsp",
        versionSource: "crates/tool/Cargo.toml#package.version",
        tagPrefix: "adocweave-lsp/v",
        assetKind: "lsp",
        assetName: "adocweave-lsp-{target}.zip",
        build: "cargo-dist",
        package: "tool-lsp",
      },
      {
        product: "browser",
        versionSource: "web/package.json#version",
        tagPrefix: "adocweave-browser/v",
        assetKind: "browser",
        assetName: "adocweave-browser-{version}.tar.xz",
        build: "script",
        buildScript: "tools/package-browser.sh",
      },
      {
        product: "textlint",
        versionSource: "web/package.json#version",
        tagPrefix: "adocweave-textlint/v",
        assetKind: "textlint-plugin",
        assetName: "adocweave-textlint-{version}.tgz",
        build: "script",
        buildScript: "tools/package-textlint.sh",
      },
      {
        product: "vscode",
        versionSource: "web/package.json#version",
        tagPrefix: "adocweave-vscode/v",
        assetKind: "vscode",
        assetName: "adocweave-vscode-{version}.vsix",
        build: "script",
        buildScript: "tools/package-vscode.sh",
      },
      {
        product: "zed",
        versionSource: "web/package.json#version",
        tagPrefix: "adocweave-zed/v",
        assetKind: "zed",
        assetName: "adocweave-zed-{version}.tar.xz",
        build: "script",
        buildScript: "tools/package-zed.sh",
      },
    ],
    targets: [
      { archive: "zip", executableSuffix: "", triple: "a-target" },
      { archive: "zip", executableSuffix: "", triple: "b-target" },
    ],
    releaseMetadata: [
      { name: "adocweave-dist-manifest.json" },
      { name: "sha256.sum" },
    ],
  };
  writeFileSync(join(root, "release/distribution-plan.json"), `${JSON.stringify(plan)}\n`);
  return { cleanup: () => rmSync(root, { force: true, recursive: true }), plan, root };
}

test("JSONとTOMLのversionSourceから製品tagとcandidate名を一意に解決します", () => {
  const { cleanup, root } = fixture();
  try {
    const cli = productIdentity("cli", { root });
    assert.equal(cli.version, "1.2.3");
    assert.equal(cli.tag, "adocweave-cli/v1.2.3");
    assert.deepEqual(cli.assetNames, [
      "adocweave-cli-a-target.zip",
      "adocweave-cli-b-target.zip",
    ]);
    const browser = productIdentity("browser", { root });
    assert.equal(browser.version, "4.5.6");
    assert.equal(browser.tag, "adocweave-browser/v4.5.6");
    assert.deepEqual(browser.assetNames, ["adocweave-browser-4.5.6.tar.xz"]);
    const plan = loadDistributionPlan(root);
    const entry = selectProduct(plan, "cli");
    assert.equal(productVersion(entry, root), "1.2.3");
    assert.equal(productTag(entry, "1.2.3"), "adocweave-cli/v1.2.3");
    assert.deepEqual(productAssets(entry, plan, "1.2.3"), cli.assetNames);
    assert.deepEqual(productIdentity("cli", { plan, root }), cli);
  } finally {
    cleanup();
  }
});

test("repository外path、不正version、重複productとtagPrefixを拒否します", () => {
  const { cleanup, plan, root } = fixture();
  const outside = mkdtempSync(join(tmpdir(), "adocweave-product-release-outside-"));
  try {
    writeFileSync(join(outside, "version.json"), '{"version":"1.2.3"}\n');
    symlinkSync(join(outside, "version.json"), join(root, "web/outside.json"));
    assert.throws(() => readVersionSource("../outside.json#version", root), /repository外|不正/);
    assert.throws(() => readVersionSource("web/outside.json#version", root), /repository外/);
    writeFileSync(join(root, "web/package.json"), '{"version":"1.2"}\n');
    assert.throws(() => productIdentity("browser", { root }), /stable SemVer/);
    plan.products.push({ ...plan.products[0] });
    writeFileSync(join(root, "release/distribution-plan.json"), `${JSON.stringify(plan)}\n`);
    assert.throws(() => loadDistributionPlan(root), /productが重複/);
  } finally {
    cleanup();
    rmSync(outside, { force: true, recursive: true });
  }
});

test("candidateとpublication planへ他製品assetを混在させません", () => {
  const { cleanup, plan, root } = fixture();
  const candidate = join(root, "candidate");
  mkdirSync(candidate);
  const assets = ["adocweave-cli-a-target.zip", "adocweave-cli-b-target.zip"];
  for (const name of [...assets, ...plan.releaseMetadata.map((entry) => entry.name)]) {
    writeFileSync(join(candidate, name), "fixture\n");
  }
  writeFileSync(
    join(candidate, "adocweave-dist-manifest.json"),
    `${JSON.stringify({
      schemaVersion: 5,
      product: "cli",
      productVersion: "1.2.3",
      sourceCommit: "a".repeat(40),
      assets: assets.map((name) => ({ name })),
    })}\n`,
  );
  const cargoDistPlan = {
    announcement_tag: "adocweave-cli/v1.2.3",
    artifacts: Object.fromEntries(assets.map((name) => [name, { name }])),
    releases: [{ app_name: "tool" }],
  };
  const publicationPlan = createPublicationPlan("cli", cargoDistPlan, { plan, root });
  try {
    assert.equal(validateProductCandidate("cli", candidate, { plan, root }).product, "cli");
    assert.equal(
      validateProductPublication("cli", candidate, publicationPlan, { plan, root }).tag,
      "adocweave-cli/v1.2.3",
    );
    writeFileSync(join(candidate, "other-product.zip"), "other\n");
    assert.throws(
      () => validateProductCandidate("cli", candidate, { plan, root }),
      /product cli 以外/,
    );
    rmSync(join(candidate, "other-product.zip"));
    assert.throws(
      () =>
        validateProductPublication(
          "cli",
          candidate,
          {
            ...publicationPlan,
            assets: [...publicationPlan.assets, "other-product.zip"],
          },
          { plan, root },
        ),
      /product cli の契約/,
    );
  } finally {
    cleanup();
  }
});

test("全製品を同じ最小publication planへ正規化します", () => {
  const { cleanup, plan, root } = fixture();
  try {
    for (const product of ["cli", "lsp", "browser", "textlint", "vscode", "zed"]) {
      const identity = productIdentity(product, { plan, root });
      const cargoDistPlan = identity.entry.build === "cargo-dist"
        ? {
            announcement_tag: identity.tag,
            artifacts: Object.fromEntries(identity.assetNames.map((name) => [name, { name }])),
            releases: [{ app_name: identity.entry.package }],
          }
        : undefined;
      const publication = createPublicationPlan(product, cargoDistPlan, { plan, root });
      assert.deepEqual(publication, {
        announcement_tag: identity.tag,
        assets: identity.assetNames,
        notesSource: "release/notes.md",
        product,
        productVersion: identity.version,
        title: `AdocWeave ${product} ${identity.version}`,
      });
      assert.equal(validatePublicationPlan(product, publication, { plan, root }).product, product);
    }
  } finally {
    cleanup();
  }
});

test("cargo-dist planに別製品assetまたはpackageがあれば正規化を拒否します", () => {
  const { cleanup, plan, root } = fixture();
  try {
    const identity = productIdentity("lsp", { plan, root });
    const cargoDistPlan = {
      announcement_tag: identity.tag,
      artifacts: Object.fromEntries(identity.assetNames.map((name) => [name, { name }])),
      releases: [{ app_name: identity.entry.package }],
    };
    assert.throws(
      () => createPublicationPlan("lsp", {
        ...cargoDistPlan,
        artifacts: { ...cargoDistPlan.artifacts, other: { name: "other.zip" } },
      }, { plan, root }),
      /以外のartifact/,
    );
    assert.throws(
      () => createPublicationPlan("lsp", {
        ...cargoDistPlan,
        releases: [...cargoDistPlan.releases, { app_name: "tool" }],
      }, { plan, root }),
      /package/,
    );
  } finally {
    cleanup();
  }
});
