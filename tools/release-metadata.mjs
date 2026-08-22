import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";

import { cargoTreePackageKeys } from "./generate-third-party-notices.mjs";
import {
  loadDistributionPlan,
  productAssetContracts,
  productVersion,
  selectProduct,
} from "./product-release.mjs";

export const RELEASE_METADATA_TOOL_VERSION = 2;
const ROOT = new URL("../", import.meta.url);
const readJson = (path) => JSON.parse(readFileSync(new URL(path, ROOT), "utf8"));
const compareText = (left, right) => left < right ? -1 : left > right ? 1 : 0;
const escapeUnzipMember = (member) => member.replace(/[\\[\]*?]/g, "\\$&");

function fail(message) {
  throw new Error(message);
}

function canonicalJson(value) {
  const normalize = (entry) => {
    if (Array.isArray(entry)) return entry.map(normalize);
    if (entry && typeof entry === "object") {
      return Object.fromEntries(Object.keys(entry).sort().map((key) => [key, normalize(entry[key])]));
    }
    return entry;
  };
  return `${JSON.stringify(normalize(value), null, 2)}\n`;
}

function digest(algorithm, bytes) {
  return createHash(algorithm).update(bytes).digest("hex");
}

const sha1 = (bytes) => digest("sha1", bytes);
const sha256 = (bytes) => digest("sha256", bytes);

function spdxId(prefix, value) {
  return `SPDXRef-${prefix}-${sha256(value).slice(0, 16)}`;
}

function archiveFiles(path, archiveName) {
  const zip = archiveName.endsWith(".zip") || archiveName.endsWith(".vsix");
  const gzipTar = archiveName.endsWith(".tgz");
  const listArgs = zip ? ["-Z1", path] : [gzipTar ? "-tzf" : "-tJf", path];
  const members = execFileSync(zip ? "unzip" : "tar", listArgs, { encoding: "utf8" })
    .trimEnd()
    .split("\n")
    .filter(Boolean);
  if (!zip) {
    const verbose = execFileSync("tar", [gzipTar ? "-tvzf" : "-tvJf", path], { encoding: "utf8" })
      .trimEnd()
      .split("\n")
      .filter(Boolean);
    if (verbose.length !== members.length || verbose.some((line) => !["-", "d"].includes(line[0]))) {
      fail(`archive contains a symlink or unsupported member type: ${archiveName}`);
    }
  }
  const files = [];
  for (const member of members) {
    if (member.startsWith("/") || member.includes("\\") || member.split("/").includes("..")) {
      fail(`unsafe archive member in ${archiveName}: ${member}`);
    }
    if (member.endsWith("/")) continue;
    const contents = execFileSync(zip ? "unzip" : "tar", zip
      ? ["-p", path, escapeUnzipMember(member)]
      : [gzipTar ? "-xzOf" : "-xJOf", path, member], {
      maxBuffer: 64 * 1024 * 1024,
    });
    files.push({
      SPDXID: spdxId("File", `${archiveName}\0${member}`),
      checksums: [
        { algorithm: "SHA1", checksumValue: sha1(contents) },
        { algorithm: "SHA256", checksumValue: sha256(contents) },
      ],
      copyrightText: "NOASSERTION",
      fileName: `./${archiveName}!/${member}`,
      licenseConcluded: "NOASSERTION",
    });
  }
  return files.sort((left, right) => compareText(left.fileName, right.fileName));
}

function cargoPackages(manifestPath, rootPackageName, filterPlatform) {
  const args = ["metadata", "--format-version=1", "--locked"];
  if (manifestPath) args.push("--manifest-path", manifestPath);
  if (filterPlatform) args.push("--filter-platform", filterPlatform);
  const metadata = JSON.parse(execFileSync("cargo", args, {
    cwd: ROOT,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  }));
  let packageIds;
  let selectedPackages;
  if (rootPackageName && filterPlatform) {
    selectedPackages = cargoTreePackageKeys(rootPackageName, filterPlatform);
  } else if (rootPackageName) {
    const roots = metadata.packages.filter((entry) => entry.name === rootPackageName && entry.source === null);
    if (roots.length !== 1) fail(`Cargo package is not unique: ${rootPackageName}`);
    const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
    packageIds = new Set();
    const pending = [roots[0].id];
    while (pending.length > 0) {
      const id = pending.pop();
      if (packageIds.has(id)) continue;
      packageIds.add(id);
      for (const dependency of nodes.get(id)?.deps ?? []) pending.push(dependency.pkg);
    }
  }
  return metadata.packages.filter((entry) =>
    (!packageIds || packageIds.has(entry.id)) &&
    (!selectedPackages || selectedPackages.has(`${entry.name}\0${entry.version}`))
  ).map((entry) => ({
    SPDXID: spdxId("CargoPackage", `${entry.name}\0${entry.version}\0${entry.source ?? "workspace"}`),
    downloadLocation: entry.source?.startsWith("registry+")
      ? `https://crates.io/crates/${entry.name}/${entry.version}`
      : "NOASSERTION",
    externalRefs: [{
      referenceCategory: "PACKAGE-MANAGER",
      referenceLocator: `pkg:cargo/${encodeURIComponent(entry.name)}@${entry.version}`,
      referenceType: "purl",
    }],
    filesAnalyzed: false,
    copyrightText: "NOASSERTION",
    licenseConcluded: "NOASSERTION",
    licenseDeclared: entry.license ?? "NOASSERTION",
    name: entry.name,
    versionInfo: entry.version,
  })).sort((left, right) => compareText(left.SPDXID, right.SPDXID));
}

function frontendPackage() {
  const entry = readJson("web-worker/package.json");
  const dependencies = Object.entries(entry.dependencies ?? {}).sort(([left], [right]) => compareText(left, right));
  if (dependencies.length !== 0) {
    fail("frontend runtime dependencies require explicit locked SBOM support");
  }
  const [namespace, packageName] = entry.name.startsWith("@") ? entry.name.split("/", 2) : [null, entry.name];
  const purlName = namespace
    ? `${encodeURIComponent(namespace)}/${encodeURIComponent(packageName)}`
    : encodeURIComponent(packageName);
  return {
    SPDXID: spdxId("NpmPackage", `${entry.name}\0${entry.version}`),
    downloadLocation: "NOASSERTION",
    copyrightText: "NOASSERTION",
    externalRefs: [{
      referenceCategory: "PACKAGE-MANAGER",
      referenceLocator: `pkg:npm/${purlName}@${entry.version}`,
      referenceType: "purl",
    }],
    filesAnalyzed: false,
    licenseConcluded: "NOASSERTION",
    licenseDeclared: "MIT OR Apache-2.0",
    name: entry.name,
    versionInfo: entry.version,
  };
}

function textlintPluginPackage() {
  const manifest = readJson("packages/textlint-plugin-asciidoc/package.json");
  return npmPackage(manifest.name, manifest.version, manifest.license);
}

function npmPackage(name, version, license = "NOASSERTION") {
  const [namespace, packageName] = name.startsWith("@") ? name.split("/", 2) : [null, name];
  const purlName = namespace
    ? `${encodeURIComponent(namespace)}/${encodeURIComponent(packageName)}`
    : encodeURIComponent(packageName);
  return {
    SPDXID: spdxId("NpmPackage", `${name}\0${version}`),
    downloadLocation: "NOASSERTION",
    copyrightText: "NOASSERTION",
    externalRefs: [{
      referenceCategory: "PACKAGE-MANAGER",
      referenceLocator: `pkg:npm/${purlName}@${version}`,
      referenceType: "purl",
    }],
    filesAnalyzed: false,
    licenseConcluded: "NOASSERTION",
    licenseDeclared: license,
    name,
    versionInfo: version,
  };
}

function vscodePackages() {
  const packageJson = readJson("editors/vscode/package.json");
  const lock = readJson("editors/vscode/package-lock.json");
  if (lock.lockfileVersion !== 3 || lock.packages?.[""]?.version !== packageJson.version) {
    fail("VS Code package lock does not match its manifest");
  }
  const packages = [npmPackage(packageJson.name, packageJson.version, packageJson.license)];
  for (const [path, entry] of Object.entries(lock.packages)) {
    if (!path || entry.dev === true || typeof entry.version !== "string") continue;
    const name = entry.name ?? path.split("node_modules/").at(-1);
    if (!name) fail(`VS Code package lock entry has no name: ${path}`);
    packages.push(npmPackage(name, entry.version, entry.license));
  }
  return [...new Map(packages.map((entry) => [entry.SPDXID, entry])).values()]
    .sort((left, right) => compareText(left.SPDXID, right.SPDXID));
}

function commitTimestamp(commit) {
  const value = execFileSync("git", ["show", "-s", "--format=%cI", commit], {
    cwd: ROOT,
    encoding: "utf8",
  }).trim();
  return new Date(value).toISOString().replace(".000Z", "Z");
}

export function buildMetadata(directory, sourceCommit, product, plan = loadDistributionPlan()) {
  if (!/^[0-9a-f]{40}$/.test(sourceCommit)) fail("source commit must be a lowercase 40-character Git commit");
  const selectedProduct = selectProduct(plan, product);
  const version = productVersion(selectedProduct);
  const plannedAssets = productAssetContracts(selectedProduct, plan, version);
  const assets = plannedAssets.map((planned) => {
    const path = join(directory, planned.name);
    let bytes;
    try {
      bytes = readFileSync(path);
    } catch {
      fail(`missing release archive: ${planned.name}`);
    }
    if (bytes.length === 0) fail(`empty release archive: ${planned.name}`);
    return {
      ...planned,
      byteSize: bytes.length,
      sha256: sha256(bytes),
      path,
    };
  }).sort((left, right) => compareText(left.name, right.name));

  const distributionManifest = {
    assets: assets.map(({ path: _path, ...asset }) => asset),
    product,
    productVersion: version,
    schemaVersion: 4,
    sourceCommit,
  };

  const dependencies = product === "cli"
    ? cargoPackages(undefined, "adocweave-cli")
    : product === "lsp"
      ? cargoPackages(undefined, "adocweave-lsp")
      : product === "browser"
        ? [...cargoPackages(undefined, "adocweave-wasm", "wasm32-unknown-unknown"), frontendPackage()]
        : product === "textlint"
          ? [...cargoPackages(undefined, "adocweave-textlint-wasm", "wasm32-unknown-unknown"), textlintPluginPackage()]
          : product === "vscode"
            ? vscodePackages()
            : cargoPackages("editors/zed/Cargo.toml", "adocweave-zed");
  const archivePackages = [];
  const files = [];
  const relationships = [];
  for (const asset of assets) {
    const packageId = spdxId("Archive", asset.name);
    const archiveEntries = archiveFiles(asset.path, asset.name);
    files.push(...archiveEntries);
    archivePackages.push({
      SPDXID: packageId,
      checksums: [{ algorithm: "SHA256", checksumValue: asset.sha256 }],
      copyrightText: "NOASSERTION",
      // Artifact bytes are produced before their GitHub Release is public.
      // The stable tag alone is insufficient to prove that upload succeeded.
      downloadLocation: "NOASSERTION",
      filesAnalyzed: true,
      licenseConcluded: "NOASSERTION",
      licenseDeclared: "MIT OR Apache-2.0",
      name: asset.name,
      packageFileName: asset.name,
      packageVerificationCode: {
        packageVerificationCodeValue: sha1(archiveEntries
          .map((entry) => entry.checksums.find((checksum) => checksum.algorithm === "SHA1").checksumValue)
          .sort(compareText)
          .join("")),
      },
      versionInfo: version,
    });
    relationships.push({ spdxElementId: "SPDXRef-DOCUMENT", relationshipType: "DESCRIBES", relatedSpdxElement: packageId });
    for (const file of archiveEntries) {
      relationships.push({ spdxElementId: packageId, relationshipType: "CONTAINS", relatedSpdxElement: file.SPDXID });
    }
    for (const dependency of dependencies) {
      relationships.push({ spdxElementId: packageId, relationshipType: "DEPENDS_ON", relatedSpdxElement: dependency.SPDXID });
    }
  }
  const packages = [...archivePackages, ...new Map(dependencies.map((entry) => [entry.SPDXID, entry])).values()]
    .sort((left, right) => compareText(left.SPDXID, right.SPDXID));
  relationships.sort((left, right) =>
    compareText(
      `${left.spdxElementId}\0${left.relationshipType}\0${left.relatedSpdxElement}`,
      `${right.spdxElementId}\0${right.relationshipType}\0${right.relatedSpdxElement}`,
    ));
  const sbom = {
    SPDXID: "SPDXRef-DOCUMENT",
    creationInfo: {
      created: commitTimestamp(sourceCommit),
      creators: [`Tool: adocweave-release-metadata/${RELEASE_METADATA_TOOL_VERSION}`],
    },
    dataLicense: "CC0-1.0",
    documentNamespace: `${plan.repository}/releases/sbom/${product}/${version}/${sourceCommit}`,
    files,
    name: `adocweave-${product} ${version} release assets`,
    packages,
    relationships,
    spdxVersion: "SPDX-2.3",
  };

  const manifestText = canonicalJson(distributionManifest);
  const sbomText = canonicalJson(sbom);
  const checksums = [
    ...assets.map((asset) => [asset.name, asset.sha256]),
    ["adocweave-dist-manifest.json", sha256(manifestText)],
    ["adocweave.spdx.json", sha256(sbomText)],
  ].sort(([left], [right]) => compareText(left, right));
  const checksumText = `${checksums.map(([name, digest]) => `${digest}  ${name}`).join("\n")}\n`;
  return { manifestText, sbomText, checksumText };
}

export function writeMetadata(directory, sourceCommit, product, plan = loadDistributionPlan()) {
  const metadata = buildMetadata(directory, sourceCommit, product, plan);
  writeFileSync(join(directory, "adocweave-dist-manifest.json"), metadata.manifestText);
  writeFileSync(join(directory, "adocweave.spdx.json"), metadata.sbomText);
  writeFileSync(join(directory, "sha256.sum"), metadata.checksumText);
}

export function verifyMetadata(directory, sourceCommit, product, plan = loadDistributionPlan()) {
  const expected = buildMetadata(directory, sourceCommit, product, plan);
  for (const [name, text] of [
    ["adocweave-dist-manifest.json", expected.manifestText],
    ["adocweave.spdx.json", expected.sbomText],
    ["sha256.sum", expected.checksumText],
  ]) {
    if (readFileSync(join(directory, name), "utf8") !== text) fail(`release metadata mismatch: ${name}`);
  }
  const entries = readdirSync(directory, { withFileTypes: true });
  if (entries.some((entry) => !entry.isFile())) {
    fail("release directory must contain public asset files only");
  }
  const actual = new Set(entries.map((entry) => entry.name));
  const selectedProduct = selectProduct(plan, product);
  const expectedNames = new Set([
    ...productAssetContracts(selectedProduct, plan, productVersion(selectedProduct))
      .map((asset) => asset.name),
    ...plan.releaseMetadata.map((entry) => entry.name),
  ]);
  if (actual.size !== expectedNames.size || [...actual].some((name) => !expectedNames.has(name))) {
    fail("release directory contains a missing, duplicate, or unplanned public asset");
  }
}

function main(args) {
  const [command, product, directoryArg, commitArg] = args;
  if (!new Set(["generate", "verify"]).has(command) || !product || !directoryArg) {
    fail("usage: release-metadata.mjs generate|verify PRODUCT ARTIFACT_DIRECTORY [SOURCE_COMMIT]");
  }
  const directory = resolve(directoryArg);
  const commit = commitArg ?? execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  if (command === "generate") writeMetadata(directory, commit, product);
  else verifyMetadata(directory, commit, product);
  process.stdout.write(`release metadata ${command}d: ${product} ${basename(directory)} @ ${commit}\n`);
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
