import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { verifyTextlintPluginReproducibility } from "./verify-textlint-plugin-reproducibility.mjs";

test("完成candidateを別のsource、Cargo targetおよびnpm cacheで1回だけ再構築する", async () => {
  const root = await mkdtemp(join(tmpdir(), "adocweave-textlint-repro-test-"));
  const candidate = join(root, "candidate.tgz");
  const calls = [];
  const verified = [];
  try {
    await writeFile(candidate, "same archive bytes");
    const hash = await verifyTextlintPluginReproducibility(candidate, {
      repositoryRoot: root,
      prepareSource: async (_repositoryRoot, sourceDirectory) => {
        await mkdir(join(sourceDirectory, "packages/textlint-plugin-asciidoc"), { recursive: true });
        await writeFile(
          join(sourceDirectory, "packages/textlint-plugin-asciidoc/package.json"),
          JSON.stringify({ version: "1.2.3" }),
        );
      },
      buildPackage: async (directories) => {
        calls.push(directories);
        await mkdir(directories.outputDirectory, { recursive: true });
        await writeFile(
          join(directories.outputDirectory, "adocweave-textlint-plugin-asciidoc-1.2.3.tgz"),
          "same archive bytes",
        );
      },
      verifyPackage: async ({ archive }) => verified.push(archive),
    });
    assert.match(hash, /^[0-9a-f]{64}$/);
    assert.equal(calls.length, 1);
    assert.equal(verified.length, 1);
    assert.notEqual(verified[0], candidate);
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

test("完成candidateとclean rebuildのbyte差を拒否する", async () => {
  const root = await mkdtemp(join(tmpdir(), "adocweave-textlint-repro-test-"));
  const candidate = join(root, "candidate.tgz");
  try {
    await writeFile(candidate, "candidate archive");
    await assert.rejects(
      verifyTextlintPluginReproducibility(candidate, {
        repositoryRoot: root,
        prepareSource: async (_repositoryRoot, sourceDirectory) => {
          await mkdir(join(sourceDirectory, "packages/textlint-plugin-asciidoc"), { recursive: true });
          await writeFile(
            join(sourceDirectory, "packages/textlint-plugin-asciidoc/package.json"),
            JSON.stringify({ version: "1.2.3" }),
          );
        },
        buildPackage: async ({ outputDirectory }) => {
          await mkdir(outputDirectory, { recursive: true });
          await writeFile(
            join(outputDirectory, "adocweave-textlint-plugin-asciidoc-1.2.3.tgz"),
            "clean archive",
          );
        },
        verifyPackage: async () => {},
      }),
      /textlint candidate and clean rebuild differ/,
    );
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});
