import * as vscode from "vscode";
import {
  LanguageClient,
  type LanguageClientOptions,
  type ServerOptions,
} from "vscode-languageclient/node";

import { selectServer, type SelectedServer } from "./server-selection.js";

const STOP_TIMEOUT_MS = 5_000;
const INSTALLATION_GUIDE = vscode.Uri.parse(
  "https://github.com/KeishiS/adocweave/blob/main/docs/user-guide/release-installation.adoc#editor-language-server",
);
const OPEN_INSTALLATION_GUIDE = "Open installation guide";
const MANUAL_INSTALLATION =
  " Install adocweave-lsp and add it to PATH, or set adocweave.server.path to its absolute path.";

/**
 * 失敗の種類ごとに、利用者が次に取れる操作を示します。
 *
 * 自動取得の失敗は、手元へ導入すれば回避できます。checksumが一致しない場合は
 * 取得元の問題なので、再試行ではなく手元への導入を案内します。
 */
function installationGuidance(code: string): string {
  if (code === "language-server-not-found") return MANUAL_INSTALLATION;
  if (code.startsWith("unsupported-platform:")) {
    return ` This platform has no distributed Language Server.${MANUAL_INSTALLATION}`;
  }
  if (code.startsWith("checksum-mismatch:") || code.startsWith("checksum-entry-missing:")) {
    return ` The downloaded archive did not match its published checksum and was discarded.${MANUAL_INSTALLATION}`;
  }
  if (code.startsWith("download-failed:") || code.startsWith("release-asset-missing:")) {
    return ` The Language Server could not be downloaded.${MANUAL_INSTALLATION}`;
  }
  return "";
}

export class ServerController implements vscode.Disposable {
  readonly #output: vscode.LogOutputChannel;
  readonly #storageDirectory?: string;
  #client?: LanguageClient;
  #disposed = false;
  #generation = 0;
  #queue: Promise<void> = Promise.resolve();

  constructor(output: vscode.LogOutputChannel, storageDirectory?: string) {
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
    const selected = await selectServer({
      configuredPath,
      storageDirectory: this.#storageDirectory,
    });
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
