/**
 * 自動取得の実地検査です。
 *
 * `adocweave.server.path`を設定せず、`PATH`にも`adocweave-lsp`がない状態で拡張を起動し、
 * GitHub Releaseから取得したLanguage Serverで診断が出るまでを確かめます。
 * 単体試験は取得経路を差し替えており、実際のnetworkとfilesystemは通っていません。
 * 0.48.0はその隙間で失敗しました。
 */
import assert from "node:assert/strict";
import { existsSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

import * as vscode from "vscode";

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
  const uri = vscode.Uri.joinPath(folders[0].uri, "root.adoc");
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document);

  // 取得と起動を待つ。取得は数MBのdownloadを含むため、余裕を持たせる。
  const diagnostics = await waitFor(() => {
    const found = vscode.languages.getDiagnostics(uri);
    return found.length > 0 ? found : undefined;
  }, 120_000).catch(() => {
    const servers = existsSync(storage) ? readdirSync(storage) : ["(storage missing)"];
    throw new Error(`診断が出ませんでした。storage=${storage} entries=${servers.join(",")}`);
  });
  assert.ok(diagnostics.length > 0);

  // 取得先に、版とtargetを含むディレクトリが1つだけあること。
  const entries = readdirSync(storage).filter((entry) => entry.startsWith("adocweave-lsp-"));
  assert.equal(entries.length, 1, `取得先の内容が想定と異なります：${entries.join(",")}`);
  const executable = join(storage, entries[0] as string, "adocweave-lsp");
  assert.ok(existsSync(executable), `実行ファイルがありません：${executable}`);
  console.log(`[acquire] downloaded: ${executable} (${statSync(executable).size} bytes)`);
}
