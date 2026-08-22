import { isAbsolute } from "node:path";

import { findOnPath } from "./path-search.js";

export type ServerSource = "configured" | "path";

export interface SelectedServer {
  readonly command: string;
  readonly source: ServerSource;
}

export interface SelectionOptions {
  readonly configuredPath?: string;
}

export interface SelectionDependencies {
  readonly findOnPath: typeof findOnPath;
}

const defaultDependencies: SelectionDependencies = {
  findOnPath,
};

export async function selectServer(
  options: SelectionOptions,
  dependencies: SelectionDependencies = defaultDependencies,
): Promise<SelectedServer> {
  if (options.configuredPath) {
    if (!isAbsolute(options.configuredPath)) throw new Error("configured-server-path-not-absolute");
    return { command: options.configuredPath, source: "configured" };
  }

  const pathCandidate = await dependencies.findOnPath("adocweave-lsp");
  if (pathCandidate && isAbsolute(pathCandidate)) return { command: pathCandidate, source: "path" };
  throw new Error("language-server-not-found");
}
