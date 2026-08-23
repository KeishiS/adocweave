import { readFileSync } from "node:fs";

const ALLOWED_RUNTIME_LICENSES = new Set([
  "Apache-2.0",
  "BlueOak-1.0.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "MIT",
]);
const REGISTRY_PREFIX = "https://registry.npmjs.org/";

function packageName(path, entry) {
  if (typeof entry.name === "string" && entry.name.length > 0) return entry.name;
  const marker = "node_modules/";
  const index = path.lastIndexOf(marker);
  return index === -1 ? undefined : path.slice(index + marker.length);
}

export function vscodeRuntimePackages(manifest, lock) {
  if (manifest.private !== true || lock.lockfileVersion !== 3 ||
      lock.packages?.[""]?.version !== manifest.version) {
    throw new Error("VS Code dependencyのmanifestとlockfileが一致しません");
  }
  const packages = [];
  for (const [path, entry] of Object.entries(lock.packages)) {
    if (!path || entry.dev === true) continue;
    const name = packageName(path, entry);
    if (!name || typeof entry.version !== "string" || typeof entry.license !== "string") {
      throw new Error(`VS Code runtime dependencyの名前、versionまたはlicenseを確認できません：${path}`);
    }
    packages.push({ name, version: entry.version, license: entry.license, resolved: entry.resolved });
  }
  return packages.sort((left, right) =>
    `${left.name}\0${left.version}`.localeCompare(`${right.name}\0${right.version}`));
}

export function validateVscodeRuntimeDependencies(manifest, lock) {
  const packages = vscodeRuntimePackages(manifest, lock);
  for (const pkg of packages) {
    if (typeof pkg.resolved !== "string" || !pkg.resolved.startsWith(REGISTRY_PREFIX)) {
      throw new Error(`VS Code runtime dependencyの取得元がnpm registryではありません：${pkg.name}`);
    }
    if (!ALLOWED_RUNTIME_LICENSES.has(pkg.license)) {
      throw new Error(`VS Code runtime dependencyのlicenseが許可されていません：${pkg.name} ${pkg.license}`);
    }
  }
  return packages;
}

export function main() {
  const manifest = JSON.parse(readFileSync("editors/vscode/package.json", "utf8"));
  const lock = JSON.parse(readFileSync("editors/vscode/package-lock.json", "utf8"));
  const packages = validateVscodeRuntimeDependencies(manifest, lock);
  process.stdout.write(`VS Code runtime dependencyを検証しました：${packages.length} package。\n`);
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
