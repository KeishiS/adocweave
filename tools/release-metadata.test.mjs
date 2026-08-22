import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { plannedProductAssets, productVersion, selectProduct } from "./product-release-plan.mjs";
import { verifyMetadata, writeMetadata } from "./release-metadata.mjs";

const metadataFiles = [
  { name: "adocweave-dist-manifest.json" },
  { name: "adocweave.spdx.json" },
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
  const assets = plannedProductAssets(plan, selected, productVersion(selected));
  for (const asset of assets) {
    const archiveRoot = asset.name.replace(/\.(?:tar\.xz|zip)$/, "");
    const stage = join(root, archiveRoot);
    mkdirSync(stage);
    writeFileSync(join(stage, asset.executable ?? "index.mjs"), `${asset.name}\n`);
    if (asset.archive === "zip") {
      execFileSync("zip", ["-X", "-q", "-r", join(artifacts, asset.name), archiveRoot], { cwd: root });
    } else {
      execFileSync("tar", ["--sort=name", "--mtime=@0", "--owner=0", "--group=0", "--numeric-owner",
        "-cJf", join(artifacts, asset.name), "-C", root, archiveRoot]);
    }
  }
  return { assets, artifacts, root };
}

const commit = () => execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();

test("LSP metadataはLSP assetとAPI versionだけを記録する", () => {
  const { assets, artifacts, root } = fixture("lsp");
  try {
    writeMetadata(artifacts, commit(), "lsp", plan);
    verifyMetadata(artifacts, commit(), "lsp", plan);
    const manifest = JSON.parse(readFileSync(join(artifacts, "adocweave-dist-manifest.json"), "utf8"));
    const sbom = JSON.parse(readFileSync(join(artifacts, "adocweave.spdx.json"), "utf8"));
    const checksums = readFileSync(join(artifacts, "sha256.sum"), "utf8").trimEnd().split("\n");
    assert.equal(manifest.schemaVersion, 3);
    assert.equal(manifest.product, "lsp");
    assert.equal(manifest.productVersion, "0.46.2");
    assert.equal(manifest.lspApiVersion, 1);
    assert.deepEqual(manifest.assets.map(({ name }) => name), assets.map(({ name }) => name));
    assert.ok(manifest.assets.every(({ kind }) => kind === "lsp"));
    assert.ok(sbom.packages.some(({ name }) => name === "adocweave-lsp"));
    assert.equal(sbom.packages.some(({ name }) => name === "adocweave-cli"), false);
    assert.deepEqual(checksums.map((line) => line.slice(66)), [
      ...assets.map(({ name }) => name),
      ...metadataFiles.slice(0, 2).map(({ name }) => name),
    ].sort());
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("Browser metadataは1 archiveとその依存関係だけを記録する", () => {
  const { artifacts, root } = fixture("browser");
  try {
    writeMetadata(artifacts, commit(), "browser", plan);
    const manifest = JSON.parse(readFileSync(join(artifacts, "adocweave-dist-manifest.json"), "utf8"));
    const sbom = JSON.parse(readFileSync(join(artifacts, "adocweave.spdx.json"), "utf8"));
    assert.equal(manifest.product, "browser");
    assert.equal(manifest.assets.length, 1);
    assert.equal(manifest.lspApiVersion, undefined);
    const purls = sbom.packages.flatMap((entry) => entry.externalRefs ?? [])
      .map((entry) => entry.referenceLocator);
    assert.ok(purls.some((entry) => entry.startsWith("pkg:npm/%40adocweave/browser@")));
    assert.equal(purls.some((entry) => entry.includes("adocweave-vscode")), false);
    assert.equal(purls.some((entry) => entry.includes("textlint-plugin")), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("製品assetの空fileと余分なfileを拒否する", () => {
  const first = fixture("browser");
  try {
    writeMetadata(first.artifacts, commit(), "browser", plan);
    writeFileSync(join(first.artifacts, "unplanned.txt"), "unplanned\n");
    assert.throws(() => verifyMetadata(first.artifacts, commit(), "browser", plan), /unplanned public asset/);
  } finally {
    rmSync(first.root, { recursive: true, force: true });
  }
  const second = fixture("browser");
  try {
    writeFileSync(join(second.artifacts, second.assets[0].name), "");
    assert.throws(() => writeMetadata(second.artifacts, commit(), "browser", plan), /empty release archive/);
  } finally {
    rmSync(second.root, { recursive: true, force: true });
  }
});
