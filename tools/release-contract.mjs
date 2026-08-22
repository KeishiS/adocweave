import { readFileSync } from "node:fs";
import process from "node:process";

import {
  productAssetContracts,
  productTag,
  productVersion,
  selectProduct,
} from "./product-release.mjs";

const ROOT = new URL("../", import.meta.url);
const read = (path) => readFileSync(new URL(path, ROOT), "utf8");
const json = (path) => JSON.parse(read(path));
const DISTRIBUTION_PLAN = json("release/distribution-plan.json");
const fail = (message) => {
  throw new Error(message);
};

export const PRODUCT_TAG = /^adocweave-(cli|lsp|browser|textlint|vscode|zed)\/v(\d+\.\d+\.\d+)$/;
export function releaseFromTag(tag) {
  const match = PRODUCT_TAG.exec(tag);
  if (!match) {
    fail(`unsupported release tag: ${tag}`);
  }
  return { product: match[1], productVersion: match[2] };
}

export function canonicalJson(value) {
  const normalize = (entry) => {
    if (Array.isArray(entry)) return entry.map(normalize);
    if (entry && typeof entry === "object") {
      return Object.fromEntries(
        Object.keys(entry)
          .sort()
          .map((key) => [key, normalize(entry[key])]),
      );
    }
    return entry;
  };
  return `${JSON.stringify(normalize(value), null, 2)}\n`;
}

function productRoute(plan, product) {
  return selectProduct(plan, product);
}

function assetsForProduct(product, productVersion, targets, plan) {
  return productAssetContracts(productRoute(plan, product), { ...plan, targets }, productVersion);
}

export function expectedAssets(product, productVersion, targets) {
  return assetsForProduct(product, productVersion, targets, DISTRIBUTION_PLAN);
}

export const EXPECTED_RELEASE_METADATA = [
  { name: "adocweave-dist-manifest.json", kind: "distribution-manifest", format: "canonical-json" },
  { name: "adocweave.spdx.json", kind: "sbom", format: "spdx-json" },
  { name: "sha256.sum", kind: "checksums", format: "sha256" },
];

export function validateReleaseIdentity(product, productVersion, tag, plan) {
  const route = productRoute(plan, product);
  if (!/^\d+\.\d+\.\d+$/.test(productVersion)) fail(`invalid product version: ${productVersion}`);
  const tagged = releaseFromTag(tag);
  if (tagged.product !== product || tagged.productVersion !== productVersion) {
    fail(`release tag does not identify ${product} ${productVersion}`);
  }
  if (tag !== productTag(route, productVersion)) fail(`release tag does not use ${product} tag prefix`);
}

export function validateDistPlan(distPlan, plan, product, productVersion, tag) {
  const route = productRoute(plan, product);
  if (route.build !== "cargo-dist") fail(`cargo-dist cannot build product: ${product}`);
  validateReleaseIdentity(product, productVersion, tag, plan);
  if (distPlan.dist_version !== plan.distVersion) fail("dist plan version mismatch");
  if (distPlan.announcement_tag !== tag) fail("dist announcement tag mismatch");

  const releases = new Map(distPlan.releases.map((release) => [release.app_name, release]));
  if (releases.size !== 1 || !releases.has(route.package)) {
    fail(`dist plan must announce exactly the ${product} package`);
  }
  for (const release of releases.values()) {
    if (release.app_version !== productVersion) fail(`dist release version mismatch: ${release.app_name}`);
  }

  const planned = new Map(assetsForProduct(product, productVersion, plan.targets, plan)
    .map((asset) => [asset.name, asset]));
  for (const [name, asset] of planned) {
    const actual = distPlan.artifacts[name];
    if (!actual) fail(`dist plan is missing public artifact: ${name}`);
    if (actual.kind !== "executable-zip") fail(`native archive has unexpected dist kind: ${name}`);
    if (JSON.stringify(actual.target_triples) !== JSON.stringify([asset.target])) {
      fail(`native archive target mismatch: ${name}`);
    }
    const executables = actual.assets.filter((entry) => entry.kind === "executable").map((entry) => entry.path);
    if (JSON.stringify(executables) !== JSON.stringify([asset.executable])) {
      fail(`native archive executable mismatch: ${name}`);
    }
    const misc = actual.assets.filter((entry) => entry.kind !== "executable").map((entry) => entry.name).sort();
    if (JSON.stringify(misc) !== JSON.stringify(["LICENSE-APACHE", "LICENSE-MIT", "README.adoc", "THIRD_PARTY_NOTICES.adoc"])) {
      fail(`native archive documentation mismatch: ${name}`);
    }
  }

  const publicArchives = Object.values(distPlan.artifacts)
    .filter((artifact) => artifact.kind === "executable-zip")
    .map((artifact) => artifact.name)
    .sort();
  if (JSON.stringify(publicArchives) !== JSON.stringify([...planned.keys()].sort())) {
    fail("dist plan contains a missing or unplanned public archive");
  }

  const runnerByTarget = Object.fromEntries(
    distPlan.ci.github.artifacts_matrix.include
      .map((entry) => [entry.targets[0], entry.runner])
      .sort(([left], [right]) => left.localeCompare(right)),
  );
  const expectedRunners = Object.fromEntries(
    plan.targets
      .map((target) => [target.triple, target.runner])
      .sort(([left], [right]) => left.localeCompare(right)),
  );
  if (JSON.stringify(runnerByTarget) !== JSON.stringify(expectedRunners)) {
    fail("dist plan runner matrix must use the declared native hosts");
  }
}

export function validateDistributionManifest(manifest, plan) {
  const keys = Object.keys(manifest).sort();
  const expectedKeys = manifest.product === "lsp"
    ? ["assets", "lspApiVersion", "product", "productVersion", "schemaVersion", "sourceCommit"]
    : ["assets", "product", "productVersion", "schemaVersion", "sourceCommit"];
  if (JSON.stringify(keys) !== JSON.stringify(expectedKeys)) fail("distribution manifest has unknown or missing fields");
  if (manifest.schemaVersion !== 3) fail("distribution manifest schemaVersion must be 3");
  productRoute(plan, manifest.product);
  if (!/^\d+\.\d+\.\d+$/.test(manifest.productVersion)) fail("distribution manifest product version is invalid");
  if (manifest.product === "lsp" && (!Number.isInteger(manifest.lspApiVersion) || manifest.lspApiVersion < 1)) {
    fail("LSP distribution manifest has invalid lspApiVersion");
  }
  if (!/^[0-9a-f]{40}$/.test(manifest.sourceCommit)) fail("sourceCommit must be a lowercase 40-character Git commit");
  const expected = new Map(assetsForProduct(manifest.product, manifest.productVersion, plan.targets, plan)
    .map((asset) => [asset.name, asset]));
  const names = manifest.assets.map((asset) => asset.name);
  if (new Set(names).size !== names.length || names.some((name, index) => index && name < names[index - 1])) {
    fail("distribution assets must have unique names sorted by name");
  }
  if (names.length !== expected.size) fail("distribution manifest asset count mismatch");
  for (const asset of manifest.assets) {
    const planned = expected.get(asset.name);
    if (!planned) fail(`unplanned distribution asset: ${asset.name}`);
    for (const field of ["kind", "target", "archive", "executable"]) {
      if (asset[field] !== planned[field]) fail(`asset ${asset.name} has invalid ${field}`);
    }
    if (!Number.isInteger(asset.byteSize) || asset.byteSize < 1) fail(`asset ${asset.name} has invalid byteSize`);
    if (!/^[0-9a-f]{64}$/.test(asset.sha256)) fail(`asset ${asset.name} has invalid sha256`);
  }
}

function tomlValue(source, key) {
  const match = source.match(new RegExp(`^${key.replaceAll("-", "\\-")}\\s*=\\s*"([^"]+)"`, "m"));
  return match?.[1] ?? fail(`missing TOML field: ${key}`);
}

export function productVersions(plan) {
  return Object.fromEntries(plan.products.map((route) => [route.product, productVersion(route)]));
}

function verifyRepository() {
  const cargo = read("Cargo.toml");
  const plan = json("release/distribution-plan.json");
  const dist = read("dist-workspace.toml");
  const repository = tomlValue(cargo, "repository");

  const planKeys = ["distVersion", "products", "releaseMetadata", "repository", "schemaVersion", "targets"];
  if (JSON.stringify(Object.keys(plan).sort()) !== JSON.stringify(planKeys) || plan.schemaVersion !== 3) {
    fail("distribution plan schema mismatch");
  }
  const products = plan.products.map((route) => route.product);
  if (JSON.stringify(products) !== JSON.stringify(["cli", "lsp", "browser", "textlint", "vscode", "zed"])) {
    fail("distribution plan must declare each product exactly once");
  }
  for (const route of plan.products) {
    const commonKeys = ["archive", "assetKind", "assetName", "build", "executable", "product", "tagPrefix", "versionSource"];
    const routeKeys = route.build === "cargo-dist"
      ? [...commonKeys, "package"].sort()
      : [...commonKeys, "buildScript"].sort();
    if (JSON.stringify(Object.keys(route).sort()) !== JSON.stringify(routeKeys)) {
      fail(`distribution product ${route.product} has unknown or missing fields`);
    }
    if (route.tagPrefix !== `adocweave-${route.product}/v`) {
      fail(`distribution product ${route.product} has an invalid tag prefix`);
    }
    if (!new Set(["cargo-dist", "script"]).has(route.build)) {
      fail(`distribution product ${route.product} has an invalid build type`);
    }
    if ((route.build === "cargo-dist") !== ["cli", "lsp"].includes(route.product)) {
      fail(`distribution product ${route.product} has an invalid build route`);
    }
    if (!route.assetName.includes(route.build === "cargo-dist" ? "{target}" : "{version}")) {
      fail(`distribution product ${route.product} has an invalid asset name template`);
    }
  }
  const versions = productVersions(plan);
  const targetKeys = [
    "archive",
    "architecture",
    "executableSuffix",
    "linkage",
    "minimumOsVersion",
    "os",
    "runner",
    "triple",
  ].sort();
  const triples = plan.targets.map((target) => target.triple);
  if (new Set(triples).size !== triples.length) {
    fail("distribution targets must have unique triples");
  }
  for (const target of plan.targets) {
    if (JSON.stringify(Object.keys(target).sort()) !== JSON.stringify(targetKeys)) {
      fail(`distribution target ${target.triple ?? "<unknown>"} has unknown or missing fields`);
    }
    if (!["linux", "darwin", "win32"].includes(target.os) ||
        !["arm64", "x64"].includes(target.architecture) ||
        !["tar.xz", "zip"].includes(target.archive) ||
        !["", ".exe"].includes(target.executableSuffix) ||
        !["static", "system"].includes(target.linkage)) {
      fail(`distribution target ${target.triple} has an invalid platform contract`);
    }
    if (target.archive !== "zip" ||
        (target.os === "win32") !== (target.executableSuffix === ".exe")) {
      fail(`distribution target ${target.triple} has an invalid archive contract`);
    }
  }

  if (plan.repository !== repository) fail("distribution repository URL mismatch");
  if (plan.distVersion !== "0.31.0" || !dist.includes('cargo-dist-version = "0.31.0"')) {
    fail("dist must be pinned to 0.31.0");
  }
  if (!dist.includes('checksum = "false"')) {
    fail("dist per-archive checksums must be disabled in favor of the canonical checksum list");
  }
  if (!dist.includes('unix-archive = ".zip"') || !dist.includes('windows-archive = ".zip"')) {
    fail("native archives must use one ZIP contract on every platform");
  }
  if (/\[\[dist\.extra-artifacts\]\]/.test(dist)) fail("cargo-dist must not build global product artifacts");
  if (!dist.includes('packages = ["adocweave-cli", "adocweave-lsp"]')) {
    fail("cargo-dist package routing must contain only CLI and LSP");
  }
  if (!dist.includes('plan-jobs = ["./release-contract"]')) fail("release contract must run in the dist plan phase");
  for (const runner of [
    'global = "ubuntu-24.04"',
    ...plan.targets.map((target) => `${target.triple} = "${target.runner}"`),
  ]) {
    if (!dist.includes(runner)) fail(`dist runner mapping is missing: ${runner}`);
  }
  if (JSON.stringify(plan.releaseMetadata) !== JSON.stringify(EXPECTED_RELEASE_METADATA)) {
    fail("release metadata asset plan is not canonical");
  }
  const fixtureText = read("release/adocweave-dist-manifest.fixture.json");
  const fixture = JSON.parse(fixtureText);
  validateDistributionManifest(fixture, plan);
  if (fixture.productVersion !== versions[fixture.product]) fail("distribution manifest fixture version source mismatch");
  const lspApiVersion = Number(read("crates/adocweave-lsp/src/lib.rs")
    .match(/pub const LSP_API_VERSION: u32 = (\d+);/)?.[1]);
  if (fixture.product === "lsp" && fixture.lspApiVersion !== lspApiVersion) {
    fail("distribution manifest fixture LSP API version mismatch");
  }
  if (fixtureText !== canonicalJson(fixture)) fail("distribution manifest fixture is not canonical JSON");
  return { plan, versions };
}

export function main(args) {
  const { plan, versions } = verifyRepository();
  const tagArg = args.find((arg) => arg.startsWith("--tag="));
  const productArg = args.find((arg) => arg.startsWith("--product="));
  if (tagArg) {
    const tag = tagArg.slice(6);
    const { product, productVersion } = releaseFromTag(tag);
    if (productArg && productArg.slice(10) !== product) {
      fail(`tag does not identify requested product: ${productArg.slice(10)}`);
    }
    validateReleaseIdentity(product, productVersion, tag, plan);
    if (versions[product] !== productVersion) fail(`tag version does not match ${product} version source ${versions[product]}`);
  }
  process.stdout.write(`release contract verified: ${plan.products.length} products\n`);
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
