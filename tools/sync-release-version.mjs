import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import process from "node:process";
import { fileURLToPath } from "node:url";

import { isStableVersion, workspaceVersion } from "./release-version.mjs";

const ROOT = new URL("../", import.meta.url);

function fail(message) {
  throw new Error(message);
}

const read = (root, path) => readFileSync(new URL(path, root), "utf8");
const write = (root, path, source) => writeFileSync(new URL(path, root), source);
const render = (template, version) => template.replace("{version}", version);

function occurrences(source, literal) {
  return source.split(literal).length - 1;
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

export function validateRegistry(registry) {
  if (
    !registry || registry.schemaVersion !== 1 || !Array.isArray(registry.literals) ||
    !Array.isArray(registry.cargoLocks)
  ) {
    fail("version同期設定が不正です");
  }
  for (const target of registry.literals) {
    if (
      typeof target.path !== "string" || typeof target.template !== "string" ||
      !target.template.includes("{version}") || !Number.isInteger(target.count) || target.count < 1
    ) {
      fail("version同期対象が不正です");
    }
  }
  for (const lock of registry.cargoLocks) {
    if (typeof lock.path !== "string" || !Array.isArray(lock.packages) || lock.packages.length === 0) {
      fail("Cargo lockfileの同期対象が不正です");
    }
  }
  return registry;
}

function validateCargoLock(source, lock, version) {
  const blocks = source.split(/\n\n/u);
  for (const packageName of lock.packages) {
    const matches = blocks.filter((block) => new RegExp(`^name = "${packageName}"$`, "m").test(block));
    if (matches.length !== 1 || /^source = /mu.test(matches[0])) {
      fail(`${lock.path}のlocal package ${packageName}が一意ではありません`);
    }
    if (occurrences(matches[0], `version = "${version}"`) !== 1) {
      fail(`${lock.path}の${packageName}がworkspace版${version}と一致しません`);
    }
  }
}

export function checkReleaseVersion({ root = ROOT, registry }) {
  validateRegistry(registry);
  const version = workspaceVersion(root);
  for (const target of registry.literals) {
    const actual = occurrences(read(root, target.path), render(target.template, version));
    if (actual !== target.count) {
      fail(`${target.path}のversion記録数が不正です：期待${target.count}件、実際${actual}件`);
    }
  }
  for (const lock of registry.cargoLocks) {
    validateCargoLock(read(root, lock.path), lock, version);
  }
  return version;
}

function updateLiteral(source, template, current, next, count, path) {
  const from = render(template, current);
  const actual = occurrences(source, from);
  if (actual !== count) fail(`${path}を更新できません：期待${count}件、実際${actual}件`);
  return source.split(from).join(render(template, next));
}

function run(command, args, root) {
  const result = spawnSync(command, args, { cwd: fileURLToPath(root), stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) fail(`${command} ${args.join(" ")}が失敗しました`);
}

export function updateReleaseVersion({ root = ROOT, registry, version, runCommand = run }) {
  validateRegistry(registry);
  if (!isStableVersion(version ?? "")) fail(`更新先versionが不正です：${version ?? "<missing>"}`);
  const current = checkReleaseVersion({ root, registry });
  if (compareVersions(version, current) <= 0) {
    fail(`更新先versionは現在のversionより大きい必要があります：${version}`);
  }

  const manifest = read(root, "Cargo.toml");
  write(root, "Cargo.toml", updateLiteral(manifest, 'version = "{version}"', current, version, 1, "Cargo.toml"));
  for (const target of registry.literals) {
    write(
      root,
      target.path,
      updateLiteral(read(root, target.path), target.template, current, version, target.count, target.path),
    );
  }

  runCommand("cargo", ["generate-lockfile"], root);
  runCommand("cargo", ["generate-lockfile", "--manifest-path", "fuzz/Cargo.toml"], root);
  runCommand("cargo", ["generate-lockfile", "--manifest-path", "editors/zed/Cargo.toml"], root);
  const updated = checkReleaseVersion({ root, registry });
  process.stdout.write(`release versionを${current}から${updated}へ同期しました\n`);
  return { current, version: updated };
}

export function parseReleaseVersionArguments(args) {
  if (args.length === 1 && args[0] === "--check") return { mode: "check", version: undefined };
  if (args.length === 2 && args[0] === "--version") {
    return { mode: "update", version: args[1] };
  }
  fail("使用方法：node tools/sync-release-version.mjs --check | --version X.Y.Z");
}

export function main(args) {
  const options = parseReleaseVersionArguments(args);
  const registry = validateRegistry(JSON.parse(read(ROOT, "release/version-sync.json")));
  if (options.mode === "check") {
    const version = checkReleaseVersion({ registry });
    process.stdout.write(`release versionの一致を確認しました：${version}\n`);
  } else {
    updateReleaseVersion({ registry, version: options.version });
  }
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
