import { readFileSync } from "node:fs";
import process from "node:process";

import toolchains from "../toolchains.json" with { type: "json" };
import { workspaceVersion } from "./release-version.mjs";

const [path] = process.argv.slice(2);
if (!path || process.argv.length !== 3) {
  process.stderr.write("usage: node tools/verify-cargo-release-metadata.mjs ROOT_METADATA_JSON\n");
  process.exit(2);
}

try {
  const metadata = JSON.parse(readFileSync(path, "utf8"));
  const expectedNames = [
    "adocweave",
    "adocweave-cli",
    "adocweave-config",
    "adocweave-host",
    "adocweave-lsp",
    "adocweave-project",
    "adocweave-textlint",
    "adocweave-wasm",
    "adocweave-workspace",
  ];
  const packages = metadata.packages.filter((pkg) => expectedNames.includes(pkg.name));
  if (packages.length !== expectedNames.length) throw new Error("cargo metadata is missing a workspace package");
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
