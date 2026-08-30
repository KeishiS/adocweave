import { readFileSync } from "node:fs";
import process from "node:process";

import toolchains from "../toolchains.json" with { type: "json" };
import { workspaceVersion } from "./native-release-version.mjs";

const [path] = process.argv.slice(2);
if (!path || process.argv.length !== 3) {
  process.stderr.write("usage: node tools/verify-cargo-release-metadata.mjs ROOT_METADATA_JSON\n");
  process.exit(2);
}

try {
  const metadata = JSON.parse(readFileSync(path, "utf8"));
  const expectedNames = [
    "adocweave-core",
    "adocweave",
    "adocweave-lsp",
    "adocweave-project",
    "adocweave-textlint",
    "adocweave-wasm",
  ];
  const packagesById = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
  const workspaceNames = metadata.workspace_members
    .map((id) => packagesById.get(id)?.name)
    .sort();
  const expectedWorkspaceNames = [...expectedNames].sort();
  if (JSON.stringify(workspaceNames) !== JSON.stringify(expectedWorkspaceNames)) {
    throw new Error("cargo metadata workspace packages do not match the final six-package layout");
  }
  const packages = expectedNames.map((name) => metadata.packages.find((pkg) => pkg.name === name));
  const version = workspaceVersion();
  for (const pkg of packages) {
    if (pkg.version !== version) throw new Error(`${pkg.name}: cargo metadata version mismatch`);
    if (pkg.repository !== "https://github.com/KeishiS/adocweave" || pkg.homepage !== pkg.repository) {
      throw new Error(`${pkg.name}: cargo metadata repository mismatch`);
    }
    if (pkg.license !== "MIT OR Apache-2.0") throw new Error(`${pkg.name}: cargo metadata license mismatch`);
    if (!Array.isArray(pkg.publish) || pkg.publish.length !== 0) {
      throw new Error(`${pkg.name}: Cargo registry publication must be disabled`);
    }
    if (pkg.rust_version !== toolchains.rustVersion) {
      throw new Error(`${pkg.name}: cargo metadata Rust version mismatch`);
    }
  }
  process.stdout.write("cargo release metadata verified\n");
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
}
