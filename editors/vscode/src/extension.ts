import * as vscode from "vscode";

import { ServerController } from "./controller.js";

let controller: ServerController | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const output = vscode.window.createOutputChannel("AdocWeave", { log: true });
  // globalStorageUriは利用者ごとの保存領域で、workspaceを跨いで同じ実行ファイルを使えます。
  // schemeで絞りません。0.48.0はfile以外を除外していましたが、実際の環境でその条件が
  // 偽になり、自動取得が黙って無効になりました。使えるかどうかは取得処理が判断します。
  const storageDirectory = vscode.Uri.joinPath(context.globalStorageUri, "servers").fsPath;
  controller = new ServerController(output, storageDirectory);
  context.subscriptions.push(
    output,
    vscode.commands.registerCommand("adocweave.restartServer", () => controller?.restart()),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration("adocweave.server.path")) {
        void controller?.restart();
      }
    }),
  );
  await controller.restart();
}

export async function deactivate(): Promise<void> {
  const active = controller;
  controller = undefined;
  await active?.dispose();
}
