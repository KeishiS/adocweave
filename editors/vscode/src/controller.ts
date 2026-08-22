import * as vscode from "vscode";
import {
  LanguageClient,
  type LanguageClientOptions,
  type ServerOptions,
} from "vscode-languageclient/node";

import { selectServer, type SelectedServer } from "./server-selection.js";

const STOP_TIMEOUT_MS = 5_000;

export class ServerController implements vscode.Disposable {
  readonly #output: vscode.LogOutputChannel;
  #client?: LanguageClient;
  #disposed = false;
  #generation = 0;
  #queue: Promise<void> = Promise.resolve();

  constructor(output: vscode.LogOutputChannel) {
    this.#output = output;
  }

  restart(): Promise<void> {
    const generation = ++this.#generation;
    this.#queue = this.#queue
      .catch(() => undefined)
      .then(async () => {
        const selected = await this.#select(generation);
        if (!selected || this.#disposed || generation !== this.#generation) return;
        await this.#stop();
        if (!this.#disposed && generation === this.#generation)
          await this.#start(generation, selected);
      })
      .catch((error: unknown) => {
        if (generation !== this.#generation || this.#disposed) return;
        const code = error instanceof Error ? error.message : "unknown-error";
        this.#output.appendLine(`Cannot start the Language Server: ${code}`);
        const guidance =
          code === "language-server-not-found"
            ? " Install adocweave-lsp and add it to PATH, or set adocweave.server.path to its absolute path."
            : "";
        void vscode.window.showErrorMessage(
          `AdocWeave cannot start the Language Server: ${code}.${guidance}`,
        );
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
