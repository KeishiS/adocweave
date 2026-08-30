import { execFileSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { runTests } from "@vscode/test-electron";
import { unzipSync } from "fflate";

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
    process.platform === "win32" ? "adocweave.exe" : "adocweave",
  );
const server = join(
  scratch,
  process.platform === "win32" ? "adocweave-test.exe" : "adocweave-test",
);

function extensionUnderTest(): string {
  const vsix = process.env.ADOCWEAVE_TEST_VSIX;
  if (vsix === undefined) return extensionRoot;
  const destination = join(scratch, "published-extension");
  for (const [name, contents] of Object.entries(unzipSync(readFileSync(vsix)))) {
    if (!name.startsWith("extension/") || name.endsWith("/")) continue;
    const path = resolve(destination, name.slice("extension/".length));
    const fromRoot = relative(destination, path);
    if (fromRoot.startsWith("..") || resolve(destination, fromRoot) !== path) {
      throw new Error(`The published VSIX contains an unsafe entry path: ${name}`);
    }
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, contents);
  }
  const manifest = JSON.parse(
    readFileSync(join(destination, "package.json"), "utf8"),
  ) as ExtensionManifest;
  if (manifest.name !== "adocweave" || manifest.publisher !== "adocweave") {
    throw new Error("The published VSIX does not contain adocweave.adocweave");
  }
  return destination;
}

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
    extensionDevelopmentPath: extensionUnderTest(),
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
