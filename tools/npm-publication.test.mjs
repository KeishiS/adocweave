import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { checkNpmPublication } from "./npm-publication.mjs";

const identity = {
  name: "@adocweave/wasm",
  version: "1.2.3"
};

function integrity(bytes) {
  const digest = createHash("sha512").update(bytes).digest("base64");
  return `sha512-${digest}`;
}

async function fixture(candidate, response) {
  const directory = await mkdtemp(join(tmpdir(), "adocweave-npm-"));
  const candidatePath = join(directory, "package.tgz");
  await writeFile(candidatePath, candidate);
  const server = createServer(response);
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  return {
    candidatePath,
    registryUrl: `http://127.0.0.1:${address.port}`,
    close: async () => {
      await new Promise((resolve) => server.close(resolve));
      await rm(directory, { recursive: true });
    }
  };
}

function metadataResponse(candidate, overrides = {}) {
  return (request, response) => {
    assert.equal(request.url, "/%40adocweave%2Fwasm/1.2.3");
    response.setHeader("content-type", "application/json");
    response.end(JSON.stringify({
      ...identity,
      ...overrides,
      dist: { integrity: overrides.integrity ?? integrity(candidate) }
    }));
  };
}

test("未公開のversionをmissingとして報告する", async () => {
  const candidate = Buffer.from("candidate");
  const server = await fixture(candidate, (_request, response) => response.writeHead(404).end());
  try {
    assert.equal(await checkNpmPublication({
      ...identity,
      candidatePath: server.candidatePath,
      registryUrl: server.registryUrl
    }), "missing");
  } finally {
    await server.close();
  }
});

test("同じtarballが公開済みならpublishedとして報告する", async () => {
  const candidate = Buffer.from("candidate");
  const server = await fixture(candidate, metadataResponse(candidate));
  try {
    assert.equal(await checkNpmPublication({
      ...identity,
      candidatePath: server.candidatePath,
      registryUrl: server.registryUrl
    }), "published");
  } finally {
    await server.close();
  }
});

test("公開済みtarballの内容が異なる場合は拒否する", async () => {
  const candidate = Buffer.from("candidate");
  const server = await fixture(candidate, metadataResponse(candidate, {
    integrity: integrity(Buffer.from("different"))
  }));
  try {
    await assert.rejects(checkNpmPublication({
      ...identity,
      candidatePath: server.candidatePath,
      registryUrl: server.registryUrl
    }), /different bytes/u);
  } finally {
    await server.close();
  }
});

test("異なるpackage identityを拒否する", async () => {
  const candidate = Buffer.from("candidate");
  const server = await fixture(candidate, metadataResponse(candidate, { version: "9.9.9" }));
  try {
    await assert.rejects(checkNpmPublication({
      ...identity,
      candidatePath: server.candidatePath,
      registryUrl: server.registryUrl
    }), /different package identity/u);
  } finally {
    await server.close();
  }
});
