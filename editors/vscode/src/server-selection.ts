import { isAbsolute } from "node:path";

import { findOnPath } from "./path-search.js";
import { downloadServer } from "./server-download.js";

export type ServerSource = "configured" | "path" | "downloaded";

export interface SelectedServer {
  readonly command: string;
  readonly source: ServerSource;
}

export interface SelectionOptions {
  readonly configuredPath?: string;
  /** 自動取得した実行ファイルを置くディレクトリ。省略すると自動取得を行いません。 */
  readonly storageDirectory?: string;
}

export interface SelectionDependencies {
  readonly findOnPath: typeof findOnPath;
  readonly downloadServer: typeof downloadServer;
}

const defaultDependencies: SelectionDependencies = {
  findOnPath,
  downloadServer,
};

/**
 * 起動するLanguage Serverを選びます。
 *
 * 設定の絶対path、`PATH`、自動取得の順で探します。利用者が導入した実行ファイルが
 * あれば常にそちらを使い、自動取得は最後の手段です。
 */
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

  if (!options.storageDirectory) throw new Error("language-server-not-found");
  return {
    command: await dependencies.downloadServer(options.storageDirectory),
    source: "downloaded",
  };
}
