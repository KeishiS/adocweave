import {
  lstatSync,
  readFileSync,
  readdirSync,
  realpathSync,
} from "node:fs";
import process from "node:process";
import { basename, isAbsolute, relative, resolve, sep } from "node:path";

const ROOT = realpathSync(new URL("../", import.meta.url));
const VERSION = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/;
const PRODUCT = /^[a-z][a-z0-9-]*$/;
const TAG_PREFIX = /^[a-z][a-z0-9-]*\/v$/;
const SAFE_PATH = /^[A-Za-z0-9._/-]+$/;
const MANIFEST_NAME = "adocweave-dist-manifest.json";
const NOTES_SOURCE = "release/notes.md";

function fail(message) {
  throw new Error(message);
}

function tomlString(source, selector) {
  const parts = selector.split(".");
  const key = parts.pop();
  const wantedSection = parts.join(".");
  let section = "";
  const values = [];
  for (const line of source.split("\n")) {
    const heading = /^\s*\[([A-Za-z0-9_.-]+)\]\s*(?:#.*)?$/.exec(line);
    if (heading) {
      section = heading[1];
      continue;
    }
    if (section !== wantedSection) continue;
    const assignment = /^\s*([A-Za-z0-9_-]+)\s*=\s*"([^"\r\n]*)"\s*(?:#.*)?$/.exec(line);
    if (assignment?.[1] === key) values.push(assignment[2]);
  }
  if (values.length !== 1) fail(`versionSource ${selector} は一つの文字列を指す必要があります`);
  return values[0];
}

function sourceFile(root, path) {
  if (
    !path ||
    isAbsolute(path) ||
    path.includes("\\") ||
    !SAFE_PATH.test(path) ||
    path.split("/").some((component) => component === "" || component === "." || component === "..")
  ) {
    fail(`versionSource pathが不正です：${path}`);
  }
  const rootPath = realpathSync(root);
  const candidate = realpathSync(resolve(rootPath, path));
  const fromRoot = relative(rootPath, candidate);
  if (!fromRoot || fromRoot === ".." || fromRoot.startsWith(`..${sep}`) || isAbsolute(fromRoot)) {
    fail(`versionSource pathがrepository外を指しています：${path}`);
  }
  if (!lstatSync(candidate).isFile()) fail(`versionSource pathがfileではありません：${path}`);
  return candidate;
}

export function readVersionSource(versionSource, root = ROOT) {
  if (typeof versionSource !== "string") fail("versionSourceは文字列である必要があります");
  const split = versionSource.split("#");
  if (split.length !== 2 || !split[1] || !/^[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*$/.test(split[1])) {
    fail(`versionSource selectorが不正です：${versionSource}`);
  }
  const [path, selector] = split;
  const file = sourceFile(root, path);
  const source = readFileSync(file, "utf8");
  let value;
  if (file.endsWith(".json")) {
    let current = JSON.parse(source);
    for (const component of selector.split(".")) {
      if (
        current === null ||
        typeof current !== "object" ||
        Array.isArray(current) ||
        !Object.hasOwn(current, component)
      ) {
        fail(`versionSource ${selector} が見つかりません`);
      }
      current = current[component];
    }
    value = current;
  } else if (file.endsWith(".toml")) {
    value = tomlString(source, selector);
  } else {
    fail(`versionSourceのfile形式が未対応です：${path}`);
  }
  if (typeof value !== "string" || !VERSION.test(value)) {
    fail(`versionSourceがstable SemVerを指していません：${versionSource}`);
  }
  return value;
}

export function loadDistributionPlan(root = ROOT) {
  const plan = JSON.parse(
    readFileSync(resolve(root, "release/distribution-plan.json"), "utf8"),
  );
  if (plan.schemaVersion !== 3 || !Array.isArray(plan.products) || plan.products.length === 0) {
    fail("distribution planの製品schemaが未対応です");
  }
  const products = new Set();
  const prefixes = new Set();
  for (const entry of plan.products) {
    if (!entry || typeof entry !== "object" || !PRODUCT.test(entry.product ?? "")) {
      fail("distribution planに不正なproductがあります");
    }
    if (products.has(entry.product)) fail(`productが重複しています：${entry.product}`);
    products.add(entry.product);
    if (!TAG_PREFIX.test(entry.tagPrefix ?? "") || prefixes.has(entry.tagPrefix)) {
      fail(`tagPrefixが不正または重複しています：${entry.tagPrefix}`);
    }
    prefixes.add(entry.tagPrefix);
    if (
      typeof entry.assetKind !== "string" ||
      typeof entry.assetName !== "string" ||
      entry.assetName.includes("/") ||
      entry.assetName.includes("\\")
    ) {
      fail(`product ${entry.product} のasset契約が不正です`);
    }
  }
  return plan;
}

export function selectProduct(plan, product) {
  if (typeof product !== "string" || !PRODUCT.test(product)) fail("productが不正です");
  const matches = plan.products.filter((entry) => entry.product === product);
  if (matches.length !== 1) fail(`productを一意に解決できません：${product}`);
  return matches[0];
}

export function productVersion(entry, root = ROOT) {
  return readVersionSource(entry.versionSource, root);
}

export function productTag(entry, version) {
  if (!TAG_PREFIX.test(entry.tagPrefix ?? "") || !VERSION.test(version ?? "")) {
    fail(`product ${entry.product} のtagを解決できません`);
  }
  return `${entry.tagPrefix}${version}`;
}

export function productAssetContracts(entry, plan, version) {
  const targets = entry.assetName.includes("{target}") ? plan.targets : [undefined];
  if (!Array.isArray(targets) || targets.length === 0) {
    fail(`product ${entry.product} のtargetがありません`);
  }
  return targets.map((target) => {
    const name = entry.assetName
      .replaceAll("{version}", version)
      .replaceAll("{target}", target?.triple ?? "");
    if (name.includes("{") || name.includes("}") || basename(name) !== name || !SAFE_PATH.test(name)) {
      fail(`product ${entry.product} のasset名を解決できません：${name}`);
    }
    return {
      archive: target?.archive ?? entry.archive,
      executable: target
        ? entry.executable?.replaceAll("{executableSuffix}", target.executableSuffix)
        : null,
      kind: entry.assetKind,
      name,
      target: target?.triple ?? null,
    };
  }).sort((left, right) => left.name.localeCompare(right.name));
}

export function productAssets(entry, plan, version) {
  return productAssetContracts(entry, plan, version).map((asset) => asset.name);
}

export function validateDistributionManifest(manifest, plan) {
  const keys = Object.keys(manifest ?? {}).sort();
  const expectedKeys = ["assets", "product", "productVersion", "schemaVersion", "sourceCommit"];
  if (JSON.stringify(keys) !== JSON.stringify(expectedKeys)) {
    fail("distribution manifest has unknown or missing fields");
  }
  if (manifest.schemaVersion !== 4) fail("distribution manifest schemaVersion must be 4");
  const entry = selectProduct(plan, manifest.product);
  if (!VERSION.test(manifest.productVersion)) fail("distribution manifest product version is invalid");
  if (!/^[0-9a-f]{40}$/.test(manifest.sourceCommit)) {
    fail("sourceCommit must be a lowercase 40-character Git commit");
  }
  const expected = new Map(
    productAssetContracts(entry, plan, manifest.productVersion).map((asset) => [asset.name, asset]),
  );
  if (!Array.isArray(manifest.assets)) fail("distribution manifest assets must be an array");
  const names = manifest.assets.map((asset) => asset?.name);
  if (new Set(names).size !== names.length || names.some((name, index) => index && name < names[index - 1])) {
    fail("distribution assets must have unique names sorted by name");
  }
  if (names.length !== expected.size) fail("distribution manifest asset count mismatch");
  for (const asset of manifest.assets) {
    const assetKeys = Object.keys(asset ?? {}).sort();
    const expectedAssetKeys = ["archive", "byteSize", "executable", "kind", "name", "sha256", "target"];
    if (JSON.stringify(assetKeys) !== JSON.stringify(expectedAssetKeys)) {
      fail("distribution manifest asset has unknown or missing fields");
    }
    const planned = expected.get(asset.name);
    if (!planned) fail(`unplanned distribution asset: ${asset.name}`);
    for (const field of ["kind", "target", "archive", "executable"]) {
      if (asset[field] !== planned[field]) fail(`asset ${asset.name} has invalid ${field}`);
    }
    if (!Number.isInteger(asset.byteSize) || asset.byteSize < 1) {
      fail(`asset ${asset.name} has invalid byteSize`);
    }
    if (!/^[0-9a-f]{64}$/.test(asset.sha256)) fail(`asset ${asset.name} has invalid sha256`);
  }
}

export function productIdentity(product, { root = ROOT, plan = loadDistributionPlan(root) } = {}) {
  const entry = selectProduct(plan, product);
  const version = productVersion(entry, root);
  return {
    assetKind: entry.assetKind,
    assetNames: productAssets(entry, plan, version),
    entry,
    product,
    tag: productTag(entry, version),
    version,
  };
}

function canonicalPublicationPlan(resolved) {
  return {
    announcement_tag: resolved.tag,
    assets: resolved.assetNames,
    notesSource: NOTES_SOURCE,
    product: resolved.product,
    productVersion: resolved.version,
    title: `AdocWeave ${resolved.product} ${resolved.version}`,
  };
}

export function createPublicationPlan(product, cargoDistPlan, options = {}) {
  const resolved = productIdentity(product, options);
  if (resolved.entry.build === "cargo-dist") {
    if (cargoDistPlan?.announcement_tag !== resolved.tag) {
      fail(`cargo-dist planのtagがproduct ${product} と一致しません`);
    }
    const artifactNames = Object.values(cargoDistPlan.artifacts ?? {})
      .map((artifact) => artifact?.name)
      .sort();
    if (JSON.stringify(artifactNames) !== JSON.stringify(resolved.assetNames)) {
      fail(`cargo-dist planにproduct ${product} 以外のartifactがあります`);
    }
    const releases = cargoDistPlan.releases ?? [];
    if (releases.length !== 1 || releases[0].app_name !== resolved.entry.package) {
      fail(`cargo-dist planのpackageがproduct ${product} と一致しません`);
    }
  } else if (resolved.entry.build !== "script" || cargoDistPlan !== undefined) {
    fail(`product ${product} のbuild経路が不正です`);
  }
  return canonicalPublicationPlan(resolved);
}

export function validatePublicationPlan(product, publicationPlan, options = {}) {
  const resolved = productIdentity(product, options);
  const expected = canonicalPublicationPlan(resolved);
  const keys = Object.keys(publicationPlan ?? {}).sort();
  if (JSON.stringify(keys) !== JSON.stringify(Object.keys(expected).sort())) {
    fail(`publication planにproduct ${product} 以外のfieldがあります`);
  }
  if (JSON.stringify(publicationPlan) !== JSON.stringify(expected)) {
    fail(`publication planがproduct ${product} の契約と一致しません`);
  }
  return resolved;
}

export function validateProductCandidate(product, directory, options = {}) {
  const resolved = productIdentity(product, options);
  const manifest = JSON.parse(readFileSync(resolve(directory, MANIFEST_NAME), "utf8"));
  validateDistributionManifest(manifest, options.plan ?? loadDistributionPlan(options.root ?? ROOT));
  if (manifest.product !== product || manifest.productVersion !== resolved.version) {
    fail(`candidate manifestがproduct ${product} の契約と一致しません`);
  }
  const names = manifest.assets.map((asset) => asset.name).sort();
  if (JSON.stringify(names) !== JSON.stringify(resolved.assetNames)) {
    fail(`candidate manifestにproduct ${product} 以外のassetがあります`);
  }
  const metadata = (options.plan ?? loadDistributionPlan(options.root ?? ROOT)).releaseMetadata.map(
    (entry) => entry.name,
  );
  const files = readdirSync(directory).sort();
  const expectedFiles = [...resolved.assetNames, ...metadata].sort();
  if (JSON.stringify(files) !== JSON.stringify(expectedFiles)) {
    fail(`candidate directoryにproduct ${product} 以外のfileがあります`);
  }
  return resolved;
}

export function validateProductPublication(product, directory, publicationPlan, options = {}) {
  const resolved = validateProductCandidate(product, directory, options);
  validatePublicationPlan(product, publicationPlan, options);
  return resolved;
}

async function readStdinJson() {
  const chunks = [];
  for await (const chunk of process.stdin) chunks.push(chunk);
  if (chunks.length === 0) fail("cargo-dist planが標準入力にありません");
  return JSON.parse(chunks.join(""));
}

async function main(args = process.argv.slice(2)) {
  if (args[0] === "--publication-plan" && args.length === 2) {
    const resolved = productIdentity(args[1]);
    const cargoDistPlan = resolved.entry.build === "cargo-dist" ? await readStdinJson() : undefined;
    process.stdout.write(`${JSON.stringify(createPublicationPlan(args[1], cargoDistPlan))}\n`);
    return;
  }
  if (args[0] === "--verify-candidate" && args.length === 3) {
    const result = validateProductCandidate(args[1], args[2]);
    process.stdout.write(`product candidateを確認しました：${result.product} ${result.version}\n`);
    return;
  }
  if (args[0] === "--verify-publication" && args.length === 3) {
    const publicationPlan = await readStdinJson();
    const result = validateProductPublication(args[1], args[2], publicationPlan);
    process.stdout.write(`product publicationを確認しました：${result.product} ${result.tag}\n`);
    return;
  }
  if (args.length === 1) {
    process.stdout.write(`${JSON.stringify(productIdentity(args[0]))}\n`);
    return;
  }
  fail("使用方法：node tools/product-release.mjs PRODUCT | --publication-plan PRODUCT | --verify-candidate PRODUCT DIRECTORY | --verify-publication PRODUCT DIRECTORY");
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
