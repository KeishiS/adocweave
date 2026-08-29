/**
 * 自動取得の実地検査を起動します。
 *
 * `adocweave.server.path`を設定せず、`PATH`から`adocweave-lsp`が見つからない状態で拡張を
 * 起動し、GitHub Releaseからの取得と起動を確かめます。networkへ出るため、通常の
 * `npm test`には含めません。
 */
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { runTests } from "@vscode/test-electron";

import { type ExtensionManifest, readJson, supportedVSCodeFloor } from "./manifests.mts";

if (process.platform === "linux" && process.env.GITHUB_ACTIONS === "true") {
  delete process.env.LD_LIBRARY_PATH;
}

const extensionRoot = resolve(fileURLToPath(new URL("../", import.meta.url)));

/** `adocweave-lsp`を含むディレクトリを除いたPATHです。 */
function pathWithoutServer(): string {
  const delimiter = process.platform === "win32" ? ";" : ":";
  const executable = process.platform === "win32" ? "adocweave-lsp.exe" : "adocweave-lsp";
  return (process.env.PATH ?? "")
    .split(delimiter)
    .filter((directory) => directory && !existsSync(join(directory, executable)))
    .join(delimiter);
}
const scratch = mkdtempSync(join(tmpdir(), "adocweave-vscode-acquire-"));
const userData = join(scratch, "user-data");
// globalStorageUriは user-data/User/globalStorage/<publisher>.<name> に対応する。
const expectedStorage = join(
  userData,
  "User",
  "globalStorage",
  "adocweave.adocweave-vscode",
  "servers",
);

try {
  mkdirSync(join(userData, "User"), { recursive: true });
  writeFileSync(join(userData, "User", "settings.json"), "{}\n");
  await runTests({
    extensionDevelopmentPath: extensionRoot,
    extensionTestsPath: join(extensionRoot, "dist-test", "test", "acquire-suite", "index.js"),
    extensionTestsEnv: {
      ADOCWEAVE_EXPECTED_STORAGE: expectedStorage,
      // 開発環境のadocweave-lspが見つからないよう、PATHから該当ディレクトリを除く。
      // 空にするとElectron自体が起動できない。
      PATH: pathWithoutServer(),
    },
    launchArgs: [
      "--disable-extensions",
      "--disable-gpu",
      "--disable-workspace-trust",
      "--user-data-dir",
      userData,
      join(extensionRoot, "test", "fixtures", "adocweave.code-workspace"),
    ],
    version: supportedVSCodeFloor(readJson<ExtensionManifest>("package.json")),
  });
} finally {
  rmSync(scratch, { force: true, recursive: true });
}
