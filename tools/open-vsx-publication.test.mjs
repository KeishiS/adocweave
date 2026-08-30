import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { checkOpenVsxPublication } from "./open-vsx-publication.mjs";

const identity = {
  namespace: "adocweave",
  name: "adocweave",
  version: "1.2.3"
};

function checksum(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

async function fixture(candidate, response) {
  const directory = await mkdtemp(join(tmpdir(), "adocweave-open-vsx-"));
  const candidatePath = join(directory, "extension.vsix");
  await writeFile(candidatePath, candidate);
  const server = createServer(response);
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  const registryUrl = `http://127.0.0.1:${address.port}`;
  return {
    candidatePath,
    outputPath: join(directory, "published.vsix"),
    registryUrl,
    close: async () => {
      await new Promise((resolve) => server.close(resolve));
      await rm(directory, { recursive: true });
    }
  };
}

function publishedResponse(candidate, overrides = {}) {
  return (request, response) => {
    const origin = `http://${request.headers.host}`;
    const prefix = "/api/adocweave/adocweave/1.2.3/file/";
    if (request.url === "/api/adocweave/adocweave/1.2.3") {
      response.setHeader("content-type", "application/json");
      response.end(JSON.stringify({
        ...identity,
        ...overrides,
        files: {
          download: `${origin}${prefix}extension.vsix`,
          sha256: `${origin}${prefix}extension.sha256`
        }
      }));
    } else if (request.url === `${prefix}extension.vsix`) {
      if (overrides.download === null) response.writeHead(404).end();
      else response.end(overrides.download ?? candidate);
    } else if (request.url === `${prefix}extension.sha256`) {
      response.end(overrides.checksum ?? checksum(candidate));
    } else {
      response.writeHead(404).end();
    }
  };
}

test("未公開のversionをmissingとして報告する", async () => {
  const value = Buffer.from("candidate");
  const server = await fixture(value, (_request, response) => response.writeHead(404).end());
  try {
    assert.equal(await checkOpenVsxPublication({
      ...identity,
      candidatePath: server.candidatePath,
      registryUrl: server.registryUrl
    }), "missing");
  } finally {
    await server.close();
  }
});

test("同じVSIXが公開済みならpublishedとして報告する", async () => {
  const value = Buffer.from("candidate");
  const server = await fixture(value, publishedResponse(value));
  try {
    assert.equal(await checkOpenVsxPublication({
      ...identity,
      candidatePath: server.candidatePath,
      outputPath: server.outputPath,
      registryUrl: server.registryUrl
    }), "published");
    assert.deepEqual(await readFile(server.outputPath), value);
  } finally {
    await server.close();
  }
});

test("metadata公開後にVSIXを取得できない間はpendingとして報告する", async () => {
  const value = Buffer.from("candidate");
  const server = await fixture(value, publishedResponse(value, {
    download: null
  }));
  try {
    assert.equal(await checkOpenVsxPublication({
      ...identity,
      candidatePath: server.candidatePath,
      registryUrl: server.registryUrl
    }), "pending");
  } finally {
    await server.close();
  }
});

test("公開済みVSIXの内容が異なる場合は拒否する", async () => {
  const value = Buffer.from("candidate");
  const server = await fixture(value, publishedResponse(value, {
    download: Buffer.from("different")
  }));
  try {
    await assert.rejects(checkOpenVsxPublication({
      ...identity,
      candidatePath: server.candidatePath,
      registryUrl: server.registryUrl
    }), /different bytes/u);
  } finally {
    await server.close();
  }
});

test("異なるextension identityを拒否する", async () => {
  const value = Buffer.from("candidate");
  const server = await fixture(value, publishedResponse(value, { version: "9.9.9" }));
  try {
    await assert.rejects(checkOpenVsxPublication({
      ...identity,
      candidatePath: server.candidatePath,
      registryUrl: server.registryUrl
    }), /different extension identity/u);
  } finally {
    await server.close();
  }
});
