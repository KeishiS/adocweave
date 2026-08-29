/**
 * 自動取得の実地検査です。
 *
 * `adocweave.server.path`を設定せず、`PATH`にも`adocweave`がない状態で拡張を起動し、
 * GitHub Releaseと同じ応答から取得したLanguage Serverで診断が出るまでを確かめます。
 * 単体試験では通らないfilesystem、展開、実行およびLSP通信を検査します。
 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

import { zipSync } from "fflate";
import * as vscode from "vscode";

const targets = new Map<string, string>([
  ["linux\0x64", "x86_64-unknown-linux-musl"],
  ["linux\0arm64", "aarch64-unknown-linux-musl"],
  ["darwin\0arm64", "aarch64-apple-darwin"],
  ["win32\0x64", "x86_64-pc-windows-msvc"],
]);

/**
 * 現在のsourceから構築した実行ファイルを、GitHub Release APIと同じ応答で返します。
 *
 * 未公開版の検査を公開済みReleaseへ依存させず、Release JSON、checksum、archive、展開、
 * 実行およびLSP通信を一つの試験で通します。
 */
function mockReleaseFetch(
  extensionPath: string,
  version: string,
): {
  readonly fetch: typeof fetch;
  readonly requested: string[];
} {
  const target = targets.get(`${process.platform}\0${process.arch}`);
  assert.ok(target, `配布対象外の実行環境です：${process.platform} ${process.arch}`);
  const executableName = process.platform === "win32" ? "adocweave.exe" : "adocweave";
  const executablePath = resolve(extensionPath, "..", "..", "target", "debug", executableName);
  const archiveName = `adocweave-${target}.zip`;
  const archive = zipSync({ [executableName]: readFileSync(executablePath) }, { level: 0 });
  const checksum = createHash("sha256").update(archive).digest("hex");
  const checksumUrl = "https://release.invalid/sha256.sum";
  const archiveUrl = `https://release.invalid/${archiveName}`;
  const requested: string[] = [];

  return {
    requested,
    fetch: (async (input: string | URL | Request) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      requested.push(url);
      if (url.startsWith("https://api.github.com/repos/KeishiS/adocweave/releases")) {
        return new Response(
          JSON.stringify([
            {
              tag_name: `v${version}`,
              assets: [
                { name: "sha256.sum", browser_download_url: checksumUrl },
                { name: archiveName, browser_download_url: archiveUrl },
              ],
            },
          ]),
        );
      }
      if (url === checksumUrl) return new Response(`${checksum}  ${archiveName}\n`);
      if (url === archiveUrl) return new Response(archive);
      return new Response("not found", { status: 404 });
    }) as typeof fetch,
  };
}

async function waitFor<T>(read: () => T | undefined, timeout: number): Promise<T> {
  const started = Date.now();
  while (Date.now() - started < timeout) {
    const value = read();
    if (value !== undefined) return value;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
  }
  throw new Error("timeout");
}

export async function run(): Promise<void> {
  const storage = process.env.ADOCWEAVE_EXPECTED_STORAGE;
  assert.ok(storage, "ADOCWEAVE_EXPECTED_STORAGEが設定されていません");
  // 前提を出力に残す。取得を経ずに通った場合を見分けるため。
  console.log(`[acquire] PATH=${process.env.PATH ?? ""}`);
  console.log(
    `[acquire] storage before: ${existsSync(storage) ? readdirSync(storage).join(",") : "(absent)"}`,
  );

  const folders = vscode.workspace.workspaceFolders;
  assert.ok(folders?.[0], "workspace folderが初期化されていません");
  const extension = vscode.extensions.getExtension("adocweave.adocweave-vscode");
  assert.ok(extension, "検査対象の拡張が見つかりません");
  const originalFetch = globalThis.fetch;
  const release = mockReleaseFetch(extension.extensionPath, String(extension.packageJSON.version));
  globalThis.fetch = release.fetch;
  const uri = vscode.Uri.joinPath(folders[0].uri, "root.adoc");
  try {
    const document = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(document);

    const diagnostics = await waitFor(() => {
      const found = vscode.languages.getDiagnostics(uri);
      return found.length > 0 ? found : undefined;
    }, 30_000).catch(() => {
      const servers = existsSync(storage) ? readdirSync(storage) : ["(storage missing)"];
      throw new Error(`診断が出ませんでした。storage=${storage} entries=${servers.join(",")}`);
    });
    assert.ok(diagnostics.length > 0);

    // 取得先に、版とtargetを含むディレクトリが1つだけあること。
    const entries = readdirSync(storage).filter((entry) => entry.startsWith("adocweave-"));
    assert.equal(entries.length, 1, `取得先の内容が想定と異なります：${entries.join(",")}`);
    const executableName = process.platform === "win32" ? "adocweave.exe" : "adocweave";
    const executable = join(storage, entries[0] as string, executableName);
    assert.ok(existsSync(executable), `実行ファイルがありません：${executable}`);
    assert.equal(release.requested.length, 3, `取得回数が想定と異なります：${release.requested}`);
    console.log(`[acquire] downloaded: ${executable} (${statSync(executable).size} bytes)`);
  } finally {
    globalThis.fetch = originalFetch;
  }
}
