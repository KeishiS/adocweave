import { readFileSync } from "node:fs";
import process from "node:process";

import toolchains from "../toolchains.json" with { type: "json" };
import { productRelease } from "./release-policy.mjs";

const [path, zedPath] = process.argv.slice(2);
if (!path || !zedPath) {
  process.stderr.write("usage: node tools/verify-cargo-release-metadata.mjs ROOT_METADATA_JSON ZED_METADATA_JSON\n");
  process.exit(2);
}

try {
  const metadata = JSON.parse(readFileSync(path, "utf8"));
  const zedMetadata = JSON.parse(readFileSync(zedPath, "utf8"));
  const expectedNames = [
    "adocweave",
    "adocweave-cli",
    "adocweave-host",
    "adocweave-lsp",
    "adocweave-textlint",
    "adocweave-textlint-wasm",
    "adocweave-wasm",
  ];
  const packages = metadata.packages.filter((pkg) => expectedNames.includes(pkg.name));
  if (packages.length !== expectedNames.length) throw new Error("cargo metadata is missing a workspace package");
  const workspaceSource = readFileSync(new URL("../Cargo.toml", import.meta.url), "utf8");
  const workspaceVersion = /^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m.exec(workspaceSource)?.[1];
  if (!workspaceVersion) throw new Error("workspace package version is missing");
  const versions = {
    adocweave: workspaceVersion,
    "adocweave-cli": productRelease("cli").version,
    "adocweave-host": workspaceVersion,
    "adocweave-lsp": productRelease("lsp").version,
    "adocweave-textlint": workspaceVersion,
    "adocweave-textlint-wasm": productRelease("textlint").version,
    "adocweave-wasm": productRelease("wasm").version,
  };
  for (const pkg of packages) {
    if (pkg.version !== versions[pkg.name]) throw new Error(`${pkg.name}: cargo metadata version mismatch`);
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
  const zed = zedMetadata.packages.find((pkg) => pkg.name === "adocweave-zed");
  if (!zed) throw new Error("cargo metadata is missing the Zed package");
  if (zed.rust_version !== toolchains.rustVersion) {
    throw new Error("adocweave-zed: cargo metadata Rust version mismatch");
  }
  process.stdout.write("cargo release metadata verified\n");
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
}
