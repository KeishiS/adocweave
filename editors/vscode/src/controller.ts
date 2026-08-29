import * as vscode from "vscode";
import {
  LanguageClient,
  type LanguageClientOptions,
  type ServerOptions,
} from "vscode-languageclient/node";

import { findOnPath } from "./path-search.js";
import { downloadServer } from "./server-download.js";
import { selectServer, type SelectedServer } from "./server-selection.js";

const STOP_TIMEOUT_MS = 5_000;
const INSTALLATION_GUIDE = vscode.Uri.parse(
  "https://github.com/KeishiS/adocweave/blob/main/docs/user-guide/release-installation.adoc#editor-language-server",
);
const OPEN_INSTALLATION_GUIDE = "Open installation guide";
const MANUAL_INSTALLATION =
  " Install adocweave-lsp and add it to PATH, or set adocweave.server.path to its absolute path.";

/**
 * 自動取得の失敗に、利用者が次に取れる操作を添えます。
 *
 * 取得の失敗はどれも手元へ導入すれば回避できます。失敗の理由はcodeそのものが
 * 示すため、種類ごとの言い換えはしません。設定pathの誤りは案内の対象外です。
 */
function installationGuidance(code: string): string {
  return code === "configured-server-path-not-absolute" ? "" : MANUAL_INSTALLATION;
}

export class ServerController implements vscode.Disposable {
  readonly #output: vscode.LogOutputChannel;
  readonly #storageDirectory: string;
  #client?: LanguageClient;
  #disposed = false;
  #generation = 0;
  #queue: Promise<void> = Promise.resolve();

  constructor(output: vscode.LogOutputChannel, storageDirectory: string) {
    this.#output = output;
    this.#storageDirectory = storageDirectory;
  }

  restart(): Promise<void> {
    const generation = ++this.#generation;
    this.#queue = this.#queue
      .catch(() => undefined)
      .then(async () => {
        await this.#stop();
        if (this.#disposed || generation !== this.#generation) return;
        const selected = await this.#select(generation);
        if (!selected || this.#disposed || generation !== this.#generation) return;
        await this.#start(generation, selected);
      })
      .catch((error: unknown) => {
        if (generation !== this.#generation || this.#disposed) return;
        const code = error instanceof Error ? error.message : "unknown-error";
        this.#output.appendLine(`Cannot start the Language Server: ${code}`);
        const guidance = installationGuidance(code);
        const message = `AdocWeave cannot start the Language Server: ${code}.${guidance}`;
        void vscode.window.showErrorMessage(message, OPEN_INSTALLATION_GUIDE).then((selection) => {
          if (selection === OPEN_INSTALLATION_GUIDE)
            void vscode.env.openExternal(INSTALLATION_GUIDE);
        });
      });
    return this.#queue;
  }

  async dispose(): Promise<void> {
    this.#disposed = true;
    ++this.#generation;
    this.#queue = this.#queue.catch(() => undefined).then(() => this.#stop());
    await this.#queue;
  }

  async #select(generation: number): Promise<SelectedServer | undefined> {
    const configuration = vscode.workspace.getConfiguration("adocweave");
    const configuredPath = configuration.get<string>("server.path")?.trim() || undefined;
    const selected = await selectServer(
      { configuredPath, storageDirectory: this.#storageDirectory },
      { findOnPath, downloadServer, log: (message) => this.#output.appendLine(message) },
    );
    if (generation !== this.#generation) return undefined;
    return selected;
  }

  async #start(generation: number, selected: SelectedServer): Promise<void> {
    const serverOptions: ServerOptions = {
      command: selected.command,
      args: [],
    };
    const clientOptions: LanguageClientOptions = {
      documentSelector: [
        { language: "asciidoc", scheme: "file" },
        { language: "asciidoc", scheme: "untitled" },
      ],
      outputChannel: this.#output,
    };
    const client = new LanguageClient(
      "adocweave",
      "AdocWeave Language Server",
      serverOptions,
      clientOptions,
    );
    this.#client = client;
    await client.start();
    if (generation !== this.#generation) {
      await this.#stop();
      return;
    }
    this.#output.appendLine(`Started the Language Server (${selected.source}).`);
  }

  async #stop(): Promise<void> {
    const client = this.#client;
    this.#client = undefined;
    if (client) {
      try {
        await client.stop(STOP_TIMEOUT_MS);
      } catch {
        this.#output.appendLine("The Language Server did not shut down within the timeout.");
      }
    }
  }
}
