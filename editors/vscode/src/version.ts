import { execFile } from "node:child_process";

const MAX_OUTPUT_BYTES = 64 * 1024;
const PROBE_TIMEOUT_MS = 5_000;

export interface ServerVersion {
  readonly lspApiVersion: number;
  readonly name: string;
  readonly packageVersion: string;
}

export function probeServerVersion(command: string): Promise<ServerVersion> {
  return new Promise((resolve, reject) => {
    execFile(
      command,
      ["--version", "--json"],
      {
        encoding: "utf8",
        maxBuffer: MAX_OUTPUT_BYTES,
        shell: false,
        timeout: PROBE_TIMEOUT_MS,
        windowsHide: true,
      },
      (error, stdout) => {
        if (error) {
          reject(new Error("server-version-probe-failed"));
          return;
        }
        let value: unknown;
        try {
          value = JSON.parse(stdout);
        } catch {
          reject(new Error("server-version-invalid-json"));
          return;
        }
        if (
          value === null ||
          typeof value !== "object" ||
          (value as Record<string, unknown>).name !== "adocweave-lsp" ||
          typeof (value as Record<string, unknown>).packageVersion !== "string" ||
          !Number.isSafeInteger((value as Record<string, unknown>).lspApiVersion) ||
          ((value as Record<string, unknown>).lspApiVersion as number) < 1
        ) {
          reject(new Error("server-version-invalid-response"));
          return;
        }
        resolve(value as ServerVersion);
      },
    );
  });
}

export async function requireCompatibleServer(
  command: string,
  supportedLspApiVersions: readonly number[],
): Promise<void> {
  const actual = await probeServerVersion(command);
  if (!supportedLspApiVersions.includes(actual.lspApiVersion)) {
    throw new Error("server-lsp-api-incompatible");
  }
}
