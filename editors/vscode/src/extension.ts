import * as vscode from "vscode";

import { ServerController } from "./controller.js";

let controller: ServerController | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const output = vscode.window.createOutputChannel("AdocWeave", { log: true });
  controller = new ServerController(output);
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
