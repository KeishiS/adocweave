import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { assertTagAbsent, resolveReleaseCandidate } from "./release-readiness.mjs";

const SHA = "a".repeat(40);

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "adocweave-readiness-"));
  mkdirSync(join(root, "release"));
  mkdirSync(join(root, "crates/lsp"), { recursive: true });
  writeFileSync(join(root, "crates/lsp/Cargo.toml"), '[package]\nversion = "1.2.3"\n');
  writeFileSync(
    join(root, "release/distribution-plan.json"),
    `${JSON.stringify({
      schemaVersion: 3,
      products: [
        {
          product: "lsp",
          versionSource: "crates/lsp/Cargo.toml#package.version",
          tagPrefix: "adocweave-lsp/v",
          assetKind: "lsp",
          assetName: "adocweave-lsp-{target}.zip",
        },
      ],
      targets: [{ triple: "test-target" }],
      releaseMetadata: [],
    })}\n`,
  );
  return { cleanup: () => rmSync(root, { force: true, recursive: true }), root };
}

function githubApi({ existingRelease = false, existingTag = false } = {}) {
  return async (input) => {
    const url = new URL(input);
    if (url.pathname.endsWith("/actions/workflows/release.yml/runs")) {
      return Response.json({
        workflow_runs: [
          { conclusion: "success", head_sha: SHA, id: 41 },
          { conclusion: "success", head_sha: SHA, id: 42 },
        ],
      });
    }
    if (url.pathname.includes("/git/ref/tags/")) {
      return existingTag ? Response.json({ ref: "tag" }) : new Response("", { status: 404 });
    }
    if (url.pathname.includes("/releases/tags/")) {
      return existingRelease ? Response.json({ id: 1 }) : new Response("", { status: 404 });
    }
    return new Response("unexpected", { status: 500 });
  };
}

test("productのversionSourceからnamespaced tagとcandidate artifactを解決します", async () => {
  const { cleanup, root } = fixture();
  try {
    const result = await resolveReleaseCandidate({
      repository: "owner/repository",
      token: "token",
      candidateSha: SHA,
      dispatchSha: SHA,
      product: "lsp",
      fetchImpl: githubApi(),
      root,
    });
    assert.deepEqual(result, {
      candidateArtifact: "release-candidate-lsp",
      candidateSha: SHA,
      product: "lsp",
      runId: 42,
      tag: "adocweave-lsp/v1.2.3",
      version: "1.2.3",
    });
  } finally {
    cleanup();
  }
});

test("別productのtagと既存tagを公開前に拒否します", async () => {
  const { cleanup, root } = fixture();
  try {
    await assert.rejects(
      assertTagAbsent({
        repository: "owner/repository",
        token: "token",
        product: "lsp",
        tag: "v1.2.3",
        fetchImpl: githubApi(),
        root,
      }),
      /product stable tagが不正/,
    );
    await assert.rejects(
      resolveReleaseCandidate({
        repository: "owner/repository",
        token: "token",
        candidateSha: SHA,
        dispatchSha: SHA,
        product: "lsp",
        fetchImpl: githubApi({ existingTag: true }),
        root,
      }),
      /すでに存在/,
    );
  } finally {
    cleanup();
  }
});
