import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { zipSync } from "fflate";

import { downloadServer, type DownloadDependencies } from "../src/server-download.js";

const target = "x86_64-unknown-linux-musl";
const archiveName = `adocweave-lsp-${target}.zip`;
const releasesUrl = "https://api.github.com/repos/KeishiS/adocweave/releases?per_page=100";
const archiveUrl = `https://example.test/${archiveName}`;
const sumsUrl = "https://example.test/sha256.sum";

const releases = JSON.stringify([
  {
    tag_name: "adocweave-lsp/v0.47.0",
    draft: false,
    prerelease: false,
    assets: [
      { name: archiveName, browser_download_url: archiveUrl },
      { name: "sha256.sum", browser_download_url: sumsUrl },
    ],
  },
]);

function archive(entries: Record<string, string> = { "adocweave-lsp": "#!/bin/sh\n" }): Uint8Array {
  const encoder = new TextEncoder();
  return zipSync(
    Object.fromEntries(Object.entries(entries).map(([name, body]) => [name, encoder.encode(body)])),
  );
}

function sums(bytes: Uint8Array, name = archiveName): string {
  return `${createHash("sha256").update(bytes).digest("hex")}  ${name}\n`;
}

function dependencies(
  bytes: Uint8Array,
  sumsBody: string,
  overrides: Partial<DownloadDependencies> = {},
): Partial<DownloadDependencies> {
  return {
    platform: "linux",
    architecture: "x64",
    fetchText: async (url) => {
      if (url === releasesUrl) return releases;
      if (url === sumsUrl) return sumsBody;
      throw new Error(`unexpected-text-url: ${url}`);
    },
    fetchBytes: async (url) => {
      if (url === archiveUrl) return bytes;
      throw new Error(`unexpected-bytes-url: ${url}`);
    },
    ...overrides,
  };
}

function withStorage(body: (storage: string) => Promise<void>): Promise<void> {
  const storage = mkdtempSync(join(tmpdir(), "adocweave-download-"));
  return body(storage).finally(() => rmSync(storage, { force: true, recursive: true }));
}

test("checksumを照合してから展開し、実行できる状態にします", async () => {
  await withStorage(async (storage) => {
    const bytes = archive();
    const executable = await downloadServer(storage, dependencies(bytes, sums(bytes)));

    assert.equal(executable, join(storage, `adocweave-lsp-0.47.0-${target}`, "adocweave-lsp"));
    assert.equal(readFileSync(executable, "utf8"), "#!/bin/sh\n");
    // 展開したままでは起動できないため、実行権限を与える。
    assert.equal(statSync(executable).mode & 0o111, 0o111);
  });
});

test("checksumが一致しない場合は展開せずに失敗します", async () => {
  await withStorage(async (storage) => {
    const bytes = archive();
    const wrong = `${"0".repeat(64)}  ${archiveName}\n`;

    await assert.rejects(
      downloadServer(storage, dependencies(bytes, wrong)),
      /^Error: checksum-mismatch: /,
    );
    // 検証に失敗したarchiveの中身を、一時的にも残さない。
    assert.deepEqual(readdirSync(storage), []);
  });
});

test("sha256.sumに対象archiveの行がない場合は失敗します", async () => {
  await withStorage(async (storage) => {
    const bytes = archive();

    await assert.rejects(
      downloadServer(storage, dependencies(bytes, sums(bytes, "other.zip"))),
      /checksum-entry-missing: adocweave-lsp-x86_64-unknown-linux-musl\.zip/,
    );
    assert.deepEqual(readdirSync(storage), []);
  });
});

test("取得済みの版を再取得しません", async () => {
  await withStorage(async (storage) => {
    const bytes = archive();
    let archiveDownloads = 0;
    const counted = dependencies(bytes, sums(bytes), {
      fetchBytes: async () => {
        archiveDownloads += 1;
        return bytes;
      },
    });

    const first = await downloadServer(storage, counted);
    const second = await downloadServer(storage, counted);

    assert.equal(first, second);
    assert.equal(archiveDownloads, 1);
  });
});

test("取得した版だけを残し、利用者のfileへ触れません", async () => {
  await withStorage(async (storage) => {
    const stale = join(storage, `adocweave-lsp-0.46.2-${target}`);
    const unrelated = join(storage, "notes.txt");
    mkdirSync(stale, { recursive: true });
    writeFileSync(join(stale, "adocweave-lsp"), "old\n");
    writeFileSync(unrelated, "keep\n");
    const bytes = archive();

    await downloadServer(storage, dependencies(bytes, sums(bytes)));

    // 更新のたびに保存領域が増え続けないよう、以前の版を消す。
    assert.deepEqual(readdirSync(storage).sort(), [`adocweave-lsp-0.47.0-${target}`, "notes.txt"]);
  });
});

test("配布していないplatformでは取得を試みません", async () => {
  await withStorage(async (storage) => {
    let requests = 0;
    await assert.rejects(
      downloadServer(storage, {
        platform: "darwin",
        architecture: "x64",
        fetchText: async () => {
          requests += 1;
          return releases;
        },
        fetchBytes: async () => {
          requests += 1;
          return new Uint8Array();
        },
      }),
      /^Error: unsupported-platform: darwin x64\. Supported targets are /,
    );
    assert.equal(requests, 0);
  });
});

test("実行ファイルを含まないarchiveを取得済みとして扱いません", async () => {
  await withStorage(async (storage) => {
    const bytes = archive({ "README.adoc": "= AdocWeave\n" });

    await assert.rejects(
      downloadServer(storage, dependencies(bytes, sums(bytes))),
      /archive-executable-missing/,
    );
    assert.deepEqual(readdirSync(storage), []);
  });
});
