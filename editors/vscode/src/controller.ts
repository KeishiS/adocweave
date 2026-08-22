import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { join } from "node:path";

import * as vscode from "vscode";
import {
  LanguageClient,
  type LanguageClientOptions,
  type ServerOptions,
  type StreamInfo,
} from "vscode-languageclient/node";

import { configuredServerPath } from "./configuration.js";
import { MANAGED_LSP_VERSION, SUPPORTED_LSP_API_VERSIONS } from "./lsp-contract.js";
import { platformForHost, type ManagedPlatform } from "./platform.js";
import { selectServer, type SelectedServer } from "./server-selection.js";

const STOP_TIMEOUT_MS = 5_000;

function waitForExit(child: ChildProcessWithoutNullStreams, timeout: number): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve();
  return new Promise((resolvePromise) => {
    const timer = setTimeout(resolvePromise, timeout);
    child.once("exit", () => {
      clearTimeout(timer);
      resolvePromise();
    });
  });
}

export class ServerController implements vscode.Disposable {
  readonly #context: vscode.ExtensionContext;
  readonly #output: vscode.LogOutputChannel;
  #abort?: AbortController;
  #child?: ChildProcessWithoutNullStreams;
  #client?: LanguageClient;
  #disposed = false;
  #generation = 0;
  #queue: Promise<void> = Promise.resolve();

  constructor(context: vscode.ExtensionContext, output: vscode.LogOutputChannel) {
    this.#context = context;
    this.#output = output;
  }

  restart(): Promise<void> {
    const generation = ++this.#generation;
    const abort = new AbortController();
    this.#abort?.abort();
    this.#abort = abort;
    this.#queue = this.#queue
      .catch(() => undefined)
      .then(async () => {
        if (abort.signal.aborted) return;
        const selected = await this.#select(generation, abort.signal);
        if (!selected || this.#disposed || generation !== this.#generation) return;
        await this.#stop();
        if (!this.#disposed && generation === this.#generation)
          await this.#start(generation, selected);
      })
      .catch((error: unknown) => {
        if (abort.signal.aborted || generation !== this.#generation || this.#disposed) return;
        const code = error instanceof Error ? error.message : "unknown-error";
        this.#output.appendLine(`Cannot start the Language Server: ${code}`);
        void vscode.window.showErrorMessage(`AdocWeave cannot start the Language Server: ${code}`);
      });
    return this.#queue;
  }

  async clearManagedServer(): Promise<void> {
    const { clearManagedServers } = await import("./installer.js");
    ++this.#generation;
    this.#abort?.abort();
    this.#queue = this.#queue
      .catch(() => undefined)
      .then(async () => {
        await this.#stop();
        await clearManagedServers(this.#managedStoragePath());
        this.#output.appendLine("Removed the managed Language Server.");
      });
    await this.#queue;
    if (!this.#disposed) await this.restart();
  }

  async dispose(): Promise<void> {
    this.#disposed = true;
    ++this.#generation;
    this.#abort?.abort();
    this.#queue = this.#queue.catch(() => undefined).then(() => this.#stop());
    await this.#queue;
  }

  async #select(generation: number, signal: AbortSignal): Promise<SelectedServer | undefined> {
    const configuration = vscode.workspace.getConfiguration("adocweave");
    const inspected = configuration.inspect<string>("server.path");
    const configured = configuredServerPath(inspected, vscode.workspace.isTrusted);
    if (configured.workspaceValueIgnored) {
      this.#output.appendLine(
        "Ignored the Language Server path configured by an untrusted workspace.",
      );
    }
    let platform: ManagedPlatform | undefined;
    try {
      platform = platformForHost();
    } catch {
      platform = undefined;
    }
    const selected = await selectServer({
      allowDownload: configuration.get<boolean>("server.download", true),
      configuredPath: configured.path,
      installer: {
        managedLspVersion: MANAGED_LSP_VERSION,
        signal,
        storagePath: this.#managedStoragePath(),
        supportedLspApiVersions: SUPPORTED_LSP_API_VERSIONS,
      },
      platform,
      warning: (code) => this.#output.appendLine(`Rejected a Language Server candidate: ${code}`),
    });
    if (generation !== this.#generation || signal.aborted) return undefined;
    return selected;
  }

  async #start(generation: number, selected: SelectedServer): Promise<void> {
    const serverOptions: ServerOptions = async (): Promise<StreamInfo> => {
      const child = spawn(selected.command, [], {
        env: process.env,
        shell: false,
        stdio: ["pipe", "pipe", "pipe"],
        windowsHide: true,
      });
      this.#child = child;
      let stderrReported = false;
      child.stderr.on("data", () => {
        if (!stderrReported) {
          stderrReported = true;
          this.#output.appendLine("The Language Server wrote to stderr.");
        }
      });
      child.once("error", () => {
        this.#output.appendLine("The Language Server process reported an error.");
      });
      return { reader: child.stdout, writer: child.stdin };
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
    const child = this.#child;
    this.#client = undefined;
    this.#child = undefined;
    if (client) {
      try {
        await client.stop(STOP_TIMEOUT_MS);
      } catch {
        this.#output.appendLine("The Language Server did not shut down within the timeout.");
      }
    }
    if (child && child.exitCode === null && child.signalCode === null) {
      child.kill();
      await waitForExit(child, 2_000);
      if (child.exitCode === null && child.signalCode === null) {
        child.kill("SIGKILL");
        await waitForExit(child, 2_000);
      }
      if (child.exitCode === null && child.signalCode === null) {
        throw new Error("language-server-process-did-not-exit");
      }
    }
  }

  #managedStoragePath(): string {
    return join(this.#context.globalStorageUri.fsPath, "servers");
  }
}
