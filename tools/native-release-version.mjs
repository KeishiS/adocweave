import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const ROOT = new URL("../", import.meta.url);
const STABLE_VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const NATIVE_PACKAGES = [
  "adocweave",
  "adocweave-cli",
  "adocweave-host",
  "adocweave-lsp",
  "adocweave-project",
  "adocweave-textlint",
  "adocweave-wasm",
  "adocweave-workspace",
];

function fail(message) {
  throw new Error(message);
}

export function workspaceVersion(root = ROOT) {
  const manifest = readFileSync(new URL("Cargo.toml", root), "utf8");
  const section = manifest.match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/u)?.[1];
  const version = section?.match(/^version\s*=\s*"([^"]+)"/mu)?.[1];
  if (!version || !STABLE_VERSION.test(version)) fail("workspace version is missing or invalid");
  return version;
}

export function releaseTag(version = workspaceVersion()) {
  if (!STABLE_VERSION.test(version)) fail(`invalid release version: ${version}`);
  return `v${version}`;
}

export function isStableVersion(version) {
  return STABLE_VERSION.test(version);
}

function occurrences(source, literal) {
  return source.split(literal).length - 1;
}

function validateCargoLock(source, path, packages, version) {
  const blocks = source.split(/\n\n/u);
  for (const packageName of packages) {
    const matches = blocks.filter((block) => new RegExp(`^name = "${packageName}"$`, "m").test(block));
    if (matches.length !== 1 || /^source = /mu.test(matches[0])) {
      fail(`${path} must contain exactly one local ${packageName} package`);
    }
    if (occurrences(matches[0], `version = "${version}"`) !== 1) {
      fail(`${path} ${packageName} does not use native version ${version}`);
    }
  }
}

export function checkNativeReleaseVersion(root = ROOT) {
  const version = workspaceVersion(root);
  const changelog = readFileSync(new URL("CHANGELOG.md", root), "utf8");
  if (occurrences(changelog, `[${version}]`) !== 2) {
    fail(`CHANGELOG.md must contain the native version ${version} exactly twice`);
  }
  validateCargoLock(
    readFileSync(new URL("Cargo.lock", root), "utf8"),
    "Cargo.lock",
    NATIVE_PACKAGES,
    version,
  );
  validateCargoLock(
    readFileSync(new URL("fuzz/Cargo.lock", root), "utf8"),
    "fuzz/Cargo.lock",
    ["adocweave"],
    version,
  );
  return version;
}

function replaceExactly(source, from, to, count, path) {
  const actual = occurrences(source, from);
  if (actual !== count) fail(`${path} contains ${actual} native version entries; expected ${count}`);
  return source.split(from).join(to);
}

function compareVersions(left, right) {
  const leftParts = left.split(".").map(BigInt);
  const rightParts = right.split(".").map(BigInt);
  for (let index = 0; index < leftParts.length; index += 1) {
    if (leftParts[index] < rightParts[index]) return -1;
    if (leftParts[index] > rightParts[index]) return 1;
  }
  return 0;
}

function runCargo(args, root) {
  const result = spawnSync("cargo", args, { cwd: fileURLToPath(root), stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) fail(`cargo ${args.join(" ")} failed`);
}

export function updateNativeReleaseVersion(version, {
  root = ROOT,
  runCommand = runCargo,
} = {}) {
  if (!isStableVersion(version ?? "")) fail(`invalid native version: ${version ?? "<missing>"}`);
  const current = checkNativeReleaseVersion(root);
  if (compareVersions(version, current) <= 0) {
    fail(`native version must be greater than ${current}: ${version}`);
  }
  const manifestUrl = new URL("Cargo.toml", root);
  writeFileSync(
    manifestUrl,
    replaceExactly(
      readFileSync(manifestUrl, "utf8"),
      `version = "${current}"`,
      `version = "${version}"`,
      1,
      "Cargo.toml",
    ),
  );
  const changelogUrl = new URL("CHANGELOG.md", root);
  writeFileSync(
    changelogUrl,
    replaceExactly(
      readFileSync(changelogUrl, "utf8"),
      `[${current}]`,
      `[${version}]`,
      2,
      "CHANGELOG.md",
    ),
  );
  runCommand(["generate-lockfile"], root);
  runCommand(["generate-lockfile", "--manifest-path", "fuzz/Cargo.toml"], root);
  return { current, version: checkNativeReleaseVersion(root) };
}

export function parseNativeVersionArguments(args) {
  if (args.length === 1 && args[0] === "--check") return { mode: "check" };
  if (args.length === 2 && args[0] === "--version") return { mode: "update", version: args[1] };
  fail("usage: node tools/native-release-version.mjs --check | --version X.Y.Z");
}

export function main(args) {
  const options = parseNativeVersionArguments(args);
  if (options.mode === "check") {
    process.stdout.write(`native release version verified: ${checkNativeReleaseVersion()}\n`);
    return;
  }
  const updated = updateNativeReleaseVersion(options.version);
  process.stdout.write(`native release version updated: ${updated.current} -> ${updated.version}\n`);
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
