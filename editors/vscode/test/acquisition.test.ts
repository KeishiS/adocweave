import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";

import {
  assetName,
  checksumAssetName,
  executableName,
  expectedChecksum,
  latestServerRelease,
  targetTriple,
  verifyChecksum,
  versionDirectory,
} from "../src/acquisition.js";

function release(tag: string, assets: readonly string[], extra: Record<string, unknown> = {}) {
  return {
    tag_name: tag,
    draft: false,
    prerelease: false,
    assets: assets.map((name) => ({
      name,
      browser_download_url: `https://example.test/${name}`,
    })),
    ...extra,
  };
}

const linuxArchive = "adocweave-lsp-x86_64-unknown-linux-musl.zip";

test("Language Serverのtagを持つ最新のstable releaseを選びます", () => {
  const body = JSON.stringify([
    release("adocweave-lsp/v0.46.2", [linuxArchive]),
    release("adocweave-wasm/v0.48.0", ["adocweave-wasm-0.48.0.tgz"]),
    release("adocweave-lsp/v0.47.0", [linuxArchive]),
  ]);

  assert.equal(latestServerRelease(body).version, "0.47.0");
});

test("ほかの製品、draftおよびprereleaseを選びません", () => {
  const body = JSON.stringify([
    release("adocweave-lsp/v0.47.0", [linuxArchive]),
    release("adocweave-lsp/v9.9.9", [], { draft: true }),
    release("adocweave-lsp/v8.8.8", [], { prerelease: true }),
  ]);

  assert.equal(latestServerRelease(body).version, "0.47.0");
  assert.throws(
    () => latestServerRelease(JSON.stringify([release("adocweave-cli/v0.47.0", [])])),
    /no-published-language-server-release/,
  );
});

test("stable SemVer以外のtagを版として扱いません", () => {
  for (const tag of [
    "adocweave-lsp/v0.47",
    "adocweave-lsp/v0.47.0.1",
    "adocweave-lsp/v0.47.0-rc.1",
  ]) {
    assert.throws(
      () => latestServerRelease(JSON.stringify([release(tag, [linuxArchive])])),
      /no-published-language-server-release/,
    );
  }
});

test("配布しているtargetだけを対応させます", () => {
  assert.equal(targetTriple("linux", "x64"), "x86_64-unknown-linux-musl");
  assert.equal(targetTriple("linux", "arm64"), "aarch64-unknown-linux-musl");
  assert.equal(targetTriple("darwin", "arm64"), "aarch64-apple-darwin");
  assert.equal(targetTriple("win32", "x64"), "x86_64-pc-windows-msvc");
  // Intel macOSとWindows ARM64のarchiveは配布していない。
  for (const [platform, architecture] of [
    ["darwin", "x64"],
    ["win32", "arm64"],
    ["freebsd", "x64"],
  ] as const) {
    assert.throws(
      () => targetTriple(platform, architecture),
      (error: Error) => {
        assert.match(error.message, /^unsupported-platform: /);
        // 利用者が次の手を選べるよう、対応する組み合わせを示す。
        assert.match(error.message, /Supported targets are linux .*macOS .*Windows /s);
        return true;
      },
    );
  }
});

test("asset名とディレクトリ名を版およびtargetから組み立てます", () => {
  assert.equal(assetName("x86_64-unknown-linux-musl"), linuxArchive);
  assert.equal(checksumAssetName(), "sha256.sum");
  assert.equal(
    versionDirectory("0.47.0", "x86_64-unknown-linux-musl"),
    "adocweave-lsp-0.47.0-x86_64-unknown-linux-musl",
  );
  assert.equal(executableName("linux"), "adocweave-lsp");
  assert.equal(executableName("win32"), "adocweave-lsp.exe");
});

test("sha256.sumから対象archiveの期待値だけを取り出します", () => {
  const sums = [
    "1111111111111111111111111111111111111111111111111111111111111111  adocweave-dist-manifest.json",
    `2222222222222222222222222222222222222222222222222222222222222222  ${linuxArchive}`,
    "3333333333333333333333333333333333333333333333333333333333333333  adocweave-lsp-aarch64-apple-darwin.zip",
  ].join("\n");

  assert.equal(
    expectedChecksum(sums, linuxArchive),
    "2222222222222222222222222222222222222222222222222222222222222222",
  );
  assert.throws(() => expectedChecksum(sums, "absent.zip"), /checksum-entry-missing: absent\.zip/);
});

test("checksumが一致しないarchiveを拒否します", () => {
  const archive = new Uint8Array([1, 2, 3, 4]);
  const digest = createHash("sha256").update(archive).digest("hex");

  assert.doesNotThrow(() => verifyChecksum(archive, digest, linuxArchive));
  assert.throws(
    () => verifyChecksum(archive, "0".repeat(64), linuxArchive),
    (error: Error) => {
      assert.match(error.message, /^checksum-mismatch: /);
      // 期待値と実際の値を残し、取り違えと改変を区別できるようにする。
      assert.ok(error.message.includes(digest));
      return true;
    },
  );
});
