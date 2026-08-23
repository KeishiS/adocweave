import { spawn } from "node:child_process";
import { readdir, readFile, realpath, rm, stat, writeFile, mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve, win32 } from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

import { satisfiesPeerRange, verifiedTextlintVersion } from "./textlint-plugin-package.mjs";

const SOURCE = `= 題名\n\n${"あ".repeat(101)}。\n`;
export const TEXTLINT_PLUGIN_ONE_SHOT = Object.freeze({
  preset: "ja-technical-writing",
  rulePackage: "textlint-rule-preset-ja-technical-writing",
  ruleVersion: "12.0.2",
});

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  const [packageSpec] = process.argv.slice(2);
  if (!packageSpec) {
    process.stderr.write("usage: node tools/textlint-plugin-npx-smoke.mjs PACKAGE_SPEC\n");
    process.exit(2);
  }
  await runTextlintPluginNpxSmoke(packageSpec);
  process.stdout.write(`textlint plugin npx smoke passed: ${basename(packageSpec)}\n`);
}

export async function runTextlintPluginNpxSmoke(
  packageSpec,
  {
    manifest,
    oneShot = TEXTLINT_PLUGIN_ONE_SHOT,
    invokeNpm = invokeNpmExec,
  } = {},
) {
  manifest ??= await loadPackageManifest();
  const normalizedPackageSpec = await normalizePackageSpec(packageSpec);
  const settings = npxSettings(manifest, oneShot);
  const root = await mkdtemp(join(tmpdir(), "adocweave-textlint-npx-smoke-"));
  try {
    const document = join(root, "document.adoc");
    await writeFile(document, SOURCE);
    const result = await invokeNpm({
      args: npxArguments(normalizedPackageSpec, settings),
      cwd: root,
      npmCache: join(root, ".npm-cache"),
    });
    if (result.code !== 1) {
      throw new Error(
        `npx smoke exited with ${result.code}\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`,
      );
    }
    assertExpectedDiagnostic(result.stdout);
    if (await readFile(document, "utf8") !== SOURCE) {
      throw new Error("npx smoke changed the AsciiDoc input");
    }
    const unexpected = (await readdir(root)).filter((name) =>
      ![".npm-cache", "document.adoc"].includes(name)
    );
    if (unexpected.length > 0) {
      throw new Error(`npx smoke wrote project dependencies: ${unexpected.join(", ")}`);
    }
  } finally {
    await rm(root, { force: true, maxRetries: 5, recursive: true, retryDelay: 100 });
  }
}

export async function loadPackageManifest() {
  const { loadTextlintPluginManifest } = await import(
    "./textlint-plugin-package.mjs"
  );
  return loadTextlintPluginManifest();
}

export function npxSettings(manifest, oneShot = TEXTLINT_PLUGIN_ONE_SHOT, textlintVersion = undefined) {
  const packageName = manifest?.name;
  // packageは範囲でtextlintを受け入れるが、単発実行の検査は固定した版で行う。
  const pinned = textlintVersion ?? verifiedTextlintVersion();
  const { preset, rulePackage, ruleVersion } = oneShot;
  if (typeof packageName !== "string" || !packageName.includes("textlint-plugin-") ||
      typeof manifest?.peerDependencies?.textlint !== "string" ||
      !satisfiesPeerRange(pinned, manifest.peerDependencies.textlint) ||
      typeof preset !== "string" ||
      typeof rulePackage !== "string" || typeof ruleVersion !== "string") {
    throw new Error("textlint pluginの単発実行設定が不足しています");
  }
  return {
    plugin: packageName.replace("/textlint-plugin-", "/"),
    preset,
    rulePackage: `${rulePackage}@${ruleVersion}`,
    textlint: `textlint@${pinned}`,
  };
}

export function npxArguments(packageSpec, settings) {
  return [
    "exec",
    "--yes",
    `--package=${settings.textlint}`,
    `--package=${packageSpec}`,
    `--package=${settings.rulePackage}`,
    "--",
    "textlint",
    "--no-textlintrc",
    "--plugin",
    settings.plugin,
    "--preset",
    settings.preset,
    "--format",
    "json",
    "document.adoc",
  ];
}

export function assertExpectedDiagnostic(stdout) {
  let reports;
  try {
    reports = JSON.parse(stdout);
  } catch (error) {
    throw new Error(`npx smoke did not return textlint JSON: ${error.message}`);
  }
  const messages = reports.flatMap(({ messages = [] }) => messages);
  if (!messages.some(({ ruleId, line }) =>
    (ruleId === "sentence-length" || ruleId?.endsWith("/sentence-length")) && line === 3
  )) {
    throw new Error("npx smoke did not report the expected sentence-length diagnostic");
  }
}

async function normalizePackageSpec(packageSpec) {
  let url;
  try {
    url = new URL(packageSpec);
  } catch {
    const path = await realpath(resolve(packageSpec));
    if (!(await stat(path)).isFile()) throw new Error(`package archive is not a file: ${path}`);
    return path;
  }
  if (url.protocol !== "https:") {
    throw new Error(`package URL must use HTTPS: ${packageSpec}`);
  }
  return url.href;
}

async function invokeNpmExec({ args, cwd, npmCache }) {
  const npm = npmInvocation();
  return runProcess(npm.command, [...npm.arguments, ...args], {
    cwd,
    env: {
      ...process.env,
      npm_config_cache: npmCache,
      npm_config_fund: "false",
    },
  });
}

export function npmInvocation({
  environment = process.env,
  executable = process.execPath,
  platform = process.platform,
} = {}) {
  if (platform !== "win32") return { arguments: [], command: "npm" };
  const cli = environment.npm_execpath ??
    win32.join(win32.dirname(executable), "node_modules", "npm", "bin", "npm-cli.js");
  return { arguments: [cli], command: executable };
}

function runProcess(command, arguments_, { cwd, env }) {
  return new Promise((resolveProcess, rejectProcess) => {
    const child = spawn(command, arguments_, { cwd, env, stdio: ["ignore", "pipe", "pipe"] });
    const stdout = [];
    const stderr = [];
    child.stdout.on("data", (chunk) => stdout.push(chunk));
    child.stderr.on("data", (chunk) => stderr.push(chunk));
    child.once("error", rejectProcess);
    child.once("close", (code) => resolveProcess({
      code: code ?? 128,
      stderr: Buffer.concat(stderr).toString("utf8"),
      stdout: Buffer.concat(stdout).toString("utf8"),
    }));
  });
}
