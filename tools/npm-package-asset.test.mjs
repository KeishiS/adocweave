import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  npmPackageProduct,
  resolvePackageAsset,
  tarballIntegrity,
  verifyPublishedPackage
} from "./npm-package-asset.mjs";

async function withDirectory(names, run) {
  const directory = await mkdtemp(join(tmpdir(), "adocweave-npm-asset-"));
  try {
    for (const name of names) await writeFile(join(directory, name), `${name}\n`);
    return await run(directory);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

test("npmへ公開する製品のpackage名を配布計画から導く", () => {
  assert.equal(npmPackageProduct("textlint").packageName, "@adocweave/textlint-plugin-asciidoc");
});

test("npm成果物を持たない製品を拒否する", () => {
  // browserとcliはnpmのtarball以外を配布し、vscodeはnpmへ公開しない。
  for (const product of ["browser", "cli", "lsp", "vscode", "zed"]) {
    assert.throws(() => npmPackageProduct(product), /npmへ公開/u);
  }
  assert.throws(() => npmPackageProduct("unknown"), /配布計画に製品がありません/u);
});

test("成果物とversionをdirectoryから一意に決める", async () => {
  await withDirectory(
    ["adocweave-textlint-plugin-asciidoc-0.47.0.tgz", "sha256.sum"],
    (directory) => {
      const asset = resolvePackageAsset("textlint", directory);
      assert.equal(asset.version, "0.47.0");
      assert.equal(asset.packageName, "@adocweave/textlint-plugin-asciidoc");
      assert.match(asset.path, /adocweave-textlint-plugin-asciidoc-0\.47\.0\.tgz$/u);
    }
  );
});

test("成果物が一つに定まらないdirectoryを拒否する", async () => {
  await withDirectory(["sha256.sum"], (directory) => {
    assert.throws(() => resolvePackageAsset("textlint", directory), /一つに定められません/u);
  });
  await withDirectory(
    ["adocweave-textlint-plugin-asciidoc-0.47.0.tgz", "adocweave-textlint-plugin-asciidoc-0.47.1.tgz"],
    (directory) => {
      assert.throws(() => resolvePackageAsset("textlint", directory), /一つに定められません/u);
    }
  );
});

test("公開済みpackageのintegrityがReleaseの成果物と一致することを確かめる", async () => {
  await withDirectory(["adocweave-textlint-plugin-asciidoc-0.47.0.tgz"], async (directory) => {
    const asset = resolvePackageAsset("textlint", directory);
    const integrity = tarballIntegrity(asset.path);
    const published = await verifyPublishedPackage(asset, async () => ({
      version: "0.47.0",
      dist: { integrity }
    }));
    assert.deepEqual(published, {
      packageName: "@adocweave/textlint-plugin-asciidoc",
      version: "0.47.0"
    });
  });
});

test("公開済みpackageが別のbyte列なら失敗させる", async () => {
  await withDirectory(["adocweave-textlint-plugin-asciidoc-0.47.0.tgz"], async (directory) => {
    const asset = resolvePackageAsset("textlint", directory);
    await assert.rejects(
      verifyPublishedPackage(asset, async () => ({ version: "0.47.0", dist: { integrity: "sha512-x" } })),
      /Releaseの成果物と一致しません/u
    );
    await assert.rejects(
      verifyPublishedPackage(asset, async () => ({})),
      /npmに@adocweave\/textlint-plugin-asciidoc@0\.47\.0が見つかりません/u
    );
  });
});
