import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

import { vscodeRuntimePackages } from "./verify-vscode-dependencies.mjs";

const root = fileURLToPath(new URL("..", import.meta.url));

function fail(message) {
  throw new Error(message);
}

function packageKey(pkg) {
  return `${pkg.name} ${pkg.version} ${pkg.license}`;
}

function cargoPackageKey(pkg) {
  return `${pkg.name}\0${pkg.version}`;
}

export function selectedThirdPartyPackages(metadata, selectedPackages) {
  const workspace = new Set(metadata.workspace_members);
  return metadata.packages
    .filter((pkg) => !workspace.has(pkg.id) && selectedPackages.has(cargoPackageKey(pkg)))
    .map((pkg) => {
      if (!pkg.license) fail(`${pkg.name} ${pkg.version} has no license metadata`);
      return { name: pkg.name, version: pkg.version, license: pkg.license };
    })
    .sort((left, right) => packageKey(left).localeCompare(packageKey(right)));
}

export function cargoTreePackageKeys(rootPackageName, target) {
  const result = spawnSync(
    "cargo",
    [
      "tree",
      "--locked",
      "--package",
      rootPackageName,
      "--target",
      target,
      "--edges",
      "normal",
      "--no-dedupe",
      "--prefix",
      "none",
      "--format",
      "{p}",
    ],
    { cwd: root, encoding: "utf8" },
  );
  if (result.status !== 0) fail(result.stderr || "cargo tree failed");
  return new Set(
    result.stdout
      .trimEnd()
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        const match = /^(\S+) v(\S+)(?: .*)?$/.exec(line);
        if (!match) fail(`cargo treeのpackageを解析できません: ${line}`);
        return `${match[1]}\0${match[2]}`;
      }),
  );
}

function groupedRows(packages) {
  const grouped = new Map();
  for (const pkg of packages) {
    const entries = grouped.get(pkg.license) ?? [];
    entries.push(`${pkg.name} ${pkg.version}`);
    grouped.set(pkg.license, entries);
  }
  return [...grouped]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([license, entries]) => `|${license}\n|${entries.join(", ")}`)
    .join("\n\n");
}

function table(packages, subject = "Crateとversion") {
  return `[cols="2,5",options="header"]
|===
|SPDX license expression |${subject}

${groupedRows(packages)}
|===`;
}

export function renderCargoThirdPartyNotices(packages, artifact) {
  return `= Third-party notices

このファイルには、${artifact}から到達するRust crateのSPDX license expressionと
versionを記載します。各licenseの全文と著作権表示は、crate packageおよび記載されたSPDX licenseを
参照してください。この表はAdocWeave自身の\`MIT OR Apache-2.0\` licenseを置き換えません。

${table(packages)}
`;
}

export function renderVscodeThirdPartyNotices(packages) {
  return `= Third-party notices

このファイルには、VS Code拡張へ同梱するnpm packageのSPDX licenseとversionを記載します。各licenseの全文と
著作権表示は、packageおよび記載されたSPDX licenseを参照してください。この表はAdocWeave自身の
\`MIT OR Apache-2.0\` licenseを置き換えません。

${table(packages, "npm packageとversion")}
`;
}

function cargoMetadata(args) {
  const result = spawnSync("cargo", ["metadata", "--locked", "--format-version=1", ...args], {
    cwd: root,
    encoding: "utf8",
  });
  if (result.status !== 0) fail(result.stderr || "cargo metadata failed");
  return JSON.parse(result.stdout);
}

export function cargoRuntimePackages(packageName, target) {
  const metadata = cargoMetadata(["--filter-platform", target]);
  return selectedThirdPartyPackages(metadata, cargoTreePackageKeys(packageName, target));
}

function writeNotice(outputPath, contents) {
  const output = resolve(root, outputPath);
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, contents);
}

export function generateNativeThirdPartyNotices(target, outputPath) {
  writeNotice(
    outputPath,
    renderCargoThirdPartyNotices(
      cargoRuntimePackages("adocweave", target),
      "native archiveに含める実行ファイル",
    ),
  );
}

export function generateWasmThirdPartyNotices(outputPath) {
  writeNotice(
    outputPath,
    renderCargoThirdPartyNotices(
      cargoRuntimePackages("adocweave-wasm", "wasm32-unknown-unknown"),
      "WebAssemblyパッケージ",
    ),
  );
}

export function generateTextlintPluginNotices(outputPath) {
  writeNotice(
    outputPath,
    renderCargoThirdPartyNotices(
      cargoRuntimePackages("adocweave-textlint", "wasm32-unknown-unknown"),
      "textlint用ProcessorのNode.js向けWebAssembly",
    ),
  );
}

export function generateVscodeThirdPartyNotices(outputPath) {
  const manifest = JSON.parse(readFileSync(new URL("../editors/vscode/package.json", import.meta.url), "utf8"));
  const lock = JSON.parse(readFileSync(new URL("../editors/vscode/package-lock.json", import.meta.url), "utf8"));
  writeNotice(outputPath, renderVscodeThirdPartyNotices(vscodeRuntimePackages(manifest, lock)));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const product = args[0];
  const native = product === "--native" && args.length === 3;
  const singleTarget = ["--wasm", "--textlint-plugin", "--vscode"].includes(product) && args.length === 2;
  if (!native && !singleTarget) {
    process.stderr.write(
      "usage: node tools/generate-third-party-notices.mjs (--native TARGET|--wasm|--textlint-plugin|--vscode) OUTPUT_PATH\n",
    );
    process.exit(2);
  }
  try {
    if (product === "--native") generateNativeThirdPartyNotices(args[1], args[2]);
    else if (product === "--wasm") generateWasmThirdPartyNotices(args[1]);
    else if (product === "--textlint-plugin") generateTextlintPluginNotices(args[1]);
    else generateVscodeThirdPartyNotices(args[1]);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
