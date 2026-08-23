import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { productAssetContracts, productVersion, selectProduct } from "./product-release.mjs";
import { verifyMetadata, writeMetadata } from "./release-metadata.mjs";

const metadataFiles = [
  { name: "adocweave-dist-manifest.json" },
  { name: "sha256.sum" },
];
const targets = [
  { archive: "zip", executableSuffix: "", triple: "aarch64-unknown-linux-musl" },
  { archive: "zip", executableSuffix: "", triple: "aarch64-apple-darwin" },
  { archive: "zip", executableSuffix: "", triple: "x86_64-unknown-linux-musl" },
  { archive: "zip", executableSuffix: ".exe", triple: "x86_64-pc-windows-msvc" },
];
const plan = {
  schemaVersion: 3,
  repository: "https://github.com/KeishiS/adocweave",
  releaseMetadata: metadataFiles,
  targets,
  products: [
    {
      product: "lsp",
      versionSource: "crates/adocweave-lsp/Cargo.toml#package.version",
      assetKind: "lsp",
      assetName: "adocweave-lsp-{target}.zip",
      archive: "zip",
      executable: "adocweave-lsp{executableSuffix}",
      build: "cargo-dist",
    },
    {
      product: "browser",
      versionSource: "web-worker/package.json#version",
      assetKind: "browser",
      assetName: "adocweave-browser-{version}.tar.xz",
      archive: "tar.xz",
      executable: null,
      build: "script",
    },
  ],
};

function fixture(product) {
  const root = mkdtempSync(join(tmpdir(), "adocweave-release-metadata-"));
  const artifacts = join(root, "artifacts");
  mkdirSync(artifacts);
  const selected = selectProduct(plan, product);
  const assets = productAssetContracts(selected, plan, productVersion(selected));
  for (const asset of assets) writeFileSync(join(artifacts, asset.name), `${asset.name}\n`);
  return { assets, artifacts, root };
}

const commit = () => execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();

test("manifestは製品identityと公開archive名だけを記録する", () => {
  const { assets, artifacts, root } = fixture("lsp");
  try {
    writeMetadata(artifacts, commit(), "lsp", plan);
    verifyMetadata(artifacts, commit(), "lsp", plan);
    const manifest = JSON.parse(readFileSync(join(artifacts, "adocweave-dist-manifest.json"), "utf8"));
    assert.equal(manifest.schemaVersion, 5);
    assert.equal(manifest.product, "lsp");
    assert.equal(manifest.productVersion, "0.46.2");
    assert.deepEqual(manifest.assets.map(({ name }) => name), assets.map(({ name }) => name));
    assert.ok(manifest.assets.every((asset) =>
      JSON.stringify(Object.keys(asset)) === JSON.stringify(["name"])));
    assert.equal(existsSync(join(artifacts, "adocweave.spdx.json")), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("checksumはSBOMを作らず全archiveとmanifestを名前順に記録する", () => {
  const { assets, artifacts, root } = fixture("lsp");
  try {
    writeMetadata(artifacts, commit(), "lsp", plan);
    const checksums = readFileSync(join(artifacts, "sha256.sum"), "utf8").trimEnd().split("\n");
    assert.deepEqual(checksums.map((line) => line.slice(66)), [
      ...assets.map(({ name }) => name),
      "adocweave-dist-manifest.json",
    ].sort());
    assert.equal(checksums.every((line) => /^[0-9a-f]{64}  [A-Za-z0-9._-]+$/.test(line)), true);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("空、欠落、余分または改変された成果物を拒否する", () => {
  for (const mutation of ["empty", "missing", "extra", "changed"]) {
    const { assets, artifacts, root } = fixture("browser");
    try {
      if (mutation === "empty") writeFileSync(join(artifacts, assets[0].name), "");
      if (mutation === "missing") rmSync(join(artifacts, assets[0].name));
      if (mutation === "extra") writeFileSync(join(artifacts, "unplanned.txt"), "unplanned\n");
      if (mutation !== "empty" && mutation !== "missing") {
        writeMetadata(artifacts, commit(), "browser", plan);
      }
      if (mutation === "changed") writeFileSync(join(artifacts, assets[0].name), "changed\n");
      const action = mutation === "empty" || mutation === "missing"
        ? () => writeMetadata(artifacts, commit(), "browser", plan)
        : () => verifyMetadata(artifacts, commit(), "browser", plan);
      assert.throws(action, /empty|missing|unplanned|mismatch/);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  }
});

test("manifestのsource commit改変を拒否する", () => {
  const { artifacts, root } = fixture("browser");
  try {
    writeMetadata(artifacts, commit(), "browser", plan);
    const path = join(artifacts, "adocweave-dist-manifest.json");
    const manifest = JSON.parse(readFileSync(path, "utf8"));
    manifest.sourceCommit = "0".repeat(40);
    writeFileSync(path, `${JSON.stringify(manifest, null, 2)}\n`);
    assert.throws(() => verifyMetadata(artifacts, commit(), "browser", plan), /metadata mismatch/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
