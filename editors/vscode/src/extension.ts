import * as vscode from "vscode";

import { ServerController } from "./controller.js";

let controller: ServerController | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const output = vscode.window.createOutputChannel("AdocWeave", { log: true });
  // globalStorageUriは利用者ごとの保存領域で、workspaceを跨いで同じ実行ファイルを使えます。
  // fileのschemeでない環境では自動取得を行いません。
  const storage = context.globalStorageUri;
  const storageDirectory =
    storage.scheme === "file" ? vscode.Uri.joinPath(storage, "servers").fsPath : undefined;
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
