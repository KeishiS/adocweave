import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { runTests } from "@vscode/test-electron";

import { type ExtensionManifest, readJson, supportedVSCodeFloor } from "./manifests.mts";

if (process.platform === "linux" && process.env.GITHUB_ACTIONS === "true") {
  delete process.env.LD_LIBRARY_PATH;
}

const extensionRoot = resolve(fileURLToPath(new URL("../", import.meta.url)));
const repositoryRoot = resolve(extensionRoot, "../..");
const scratch = mkdtempSync(join(tmpdir(), "adocweave-vscode-host-"));
const userData = join(scratch, "user-data");
const sourceServer =
  process.env.ADOCWEAVE_TEST_SERVER ??
  join(
    repositoryRoot,
    "target",
    "debug",
    process.platform === "win32" ? "adocweave-lsp.exe" : "adocweave-lsp",
  );
const server = join(
  scratch,
  process.platform === "win32" ? "adocweave-lsp-test.exe" : "adocweave-lsp-test",
);

/** Command lines of every running process, one per line. */
function runningProcesses(): string {
  if (process.platform === "win32") {
    return execFileSync(
      "powershell.exe",
      [
        "-NoProfile",
        "-Command",
        "Get-CimInstance Win32_Process | Select-Object -ExpandProperty CommandLine",
      ],
      { encoding: "utf8" },
    );
  }
  return execFileSync("ps", ["-eo", "args="], { encoding: "utf8" });
}

try {
  copyFileSync(sourceServer, server);
  if (process.platform !== "win32") chmodSync(server, 0o755);
  mkdirSync(join(userData, "User"), { recursive: true });
  writeFileSync(
    join(userData, "User", "settings.json"),
    `${JSON.stringify({
      "adocweave.server.path": server,
    })}\n`,
  );
  await runTests({
    extensionDevelopmentPath: extensionRoot,
    extensionTestsPath: join(extensionRoot, "dist-test", "test", "suite", "index.js"),
    launchArgs: [
      "--disable-extensions",
      "--disable-gpu",
      "--disable-workspace-trust",
      "--user-data-dir",
      userData,
      join(extensionRoot, "test", "fixtures", "adocweave.code-workspace"),
    ],
    // 対応範囲として宣言した下限で検査する。engines.vscodeから導く理由はmanifests.mtsを参照。
    version: supportedVSCodeFloor(readJson<ExtensionManifest>("package.json")),
  });
  const serverNeedle = server.toLocaleLowerCase("en-US");
  if (
    runningProcesses()
      .split(/\r?\n/)
      .some((line) => line.toLocaleLowerCase("en-US").includes(serverNeedle))
  ) {
    throw new Error("A Language Server process is still running after the extension host exited");
  }
} finally {
  rmSync(scratch, { force: true, recursive: true });
}
