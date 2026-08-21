import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import {
  downloadAndUnzipVSCode,
  resolveCliArgsFromVSCodeExecutablePath,
} from "@vscode/test-electron";
import { unzipSync, zipSync } from "fflate";

import { type ExtensionManifest, readJson } from "./manifests.mts";

if (process.platform === "linux" && process.env.GITHUB_ACTIONS === "true") {
  delete process.env.LD_LIBRARY_PATH;
}

const packageJson = readJson<ExtensionManifest>("package.json");
const baseline = resolve("../../target/distrib", `adocweave-vscode-${packageJson.version}.vsix`);
const extensionId = `${packageJson.publisher}.${packageJson.name}`;
const scratch = mkdtempSync(join(tmpdir(), "adocweave-vsix-install-"));
const extensionsDirectory = join(scratch, "extensions");
const userDataDirectory = join(scratch, "user-data");

/** The VS Code CLI as `[command, ...leading arguments]`. */
type CliArguments = readonly string[];

/**
 * Copies the baseline VSIX with a different version so the update and
 * rollback steps exercise the real install path without a second build.
 */
function fixtureVersion(version: string): string {
  const entries = unzipSync(readFileSync(baseline));
  const manifestEntry = entries["extension/package.json"];
  const vsixManifestEntry = entries["extension.vsixmanifest"];
  if (manifestEntry === undefined || vsixManifestEntry === undefined) {
    throw new Error("The baseline VSIX does not contain its manifests");
  }
  const manifest = JSON.parse(Buffer.from(manifestEntry).toString("utf8")) as ExtensionManifest;
  manifest.version = version;
  entries["extension/package.json"] = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`);
  const vsixManifest = Buffer.from(vsixManifestEntry).toString("utf8");
  entries["extension.vsixmanifest"] = Buffer.from(
    vsixManifest.replace(/(<Identity\b[^>]*\bVersion=")[^"]+(")/, `$1${version}$2`),
  );
  const path = join(scratch, `adocweave-vscode-${version}.vsix`);
  writeFileSync(path, zipSync(entries, { level: 9 }));
  return path;
}

function runCli(baseArguments: CliArguments, arguments_: readonly string[]): string {
  const [command, ...prefix] = baseArguments;
  if (command === undefined) throw new Error("The VS Code CLI command is empty");
  const result = spawnSync(
    command,
    [
      ...prefix,
      "--extensions-dir",
      extensionsDirectory,
      "--user-data-dir",
      userDataDirectory,
      ...arguments_,
    ],
    {
      encoding: "utf8",
      shell: process.platform === "win32",
    },
  );
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || `VS Code CLI exited with ${result.status}`);
  }
  return result.stdout;
}

function installedVersion(baseArguments: CliArguments): string | undefined {
  const listed = runCli(baseArguments, ["--list-extensions", "--show-versions"]);
  return listed
    .split(/\r?\n/)
    .find((line) => line.toLowerCase().startsWith(`${extensionId.toLowerCase()}@`))
    ?.split("@")
    .at(-1);
}

try {
  mkdirSync(extensionsDirectory);
  mkdirSync(userDataDirectory);
  const executable = await downloadAndUnzipVSCode("1.125.0");
  const cli = resolveCliArgsFromVSCodeExecutablePath(executable);
  const [major, minor, patch] = packageJson.version.split(".").map(Number);
  if (major === undefined || minor === undefined || patch === undefined) {
    throw new Error(`The extension version is not MAJOR.MINOR.PATCH: ${packageJson.version}`);
  }
  const updateVersion = `${major}.${minor}.${patch + 1}`;
  const update = fixtureVersion(updateVersion);

  runCli(cli, ["--install-extension", baseline]);
  if (installedVersion(cli) !== packageJson.version) throw new Error("VSIX install failed");
  runCli(cli, ["--install-extension", update, "--force"]);
  if (installedVersion(cli) !== updateVersion) throw new Error("VSIX update failed");
  runCli(cli, ["--install-extension", baseline, "--force"]);
  if (installedVersion(cli) !== packageJson.version) throw new Error("VSIX rollback failed");
  runCli(cli, ["--uninstall-extension", extensionId]);
  if (installedVersion(cli) !== undefined) throw new Error("VSIX uninstall failed");
  if (
    readFileSync(baseline).byteLength === 0 ||
    Object.keys(unzipSync(readFileSync(baseline))).length === 0
  ) {
    throw new Error("baseline VSIX was modified");
  }
  process.stdout.write("VSIX install, update, rollback, and uninstall succeeded.\n");
} finally {
  rmSync(scratch, { force: true, recursive: true });
}

// A retried VS Code download can leave helper handles alive after every
// installation check and cleanup has completed. Do not let those unrelated
// handles keep the release gate running until its job timeout.
process.exit(0);
