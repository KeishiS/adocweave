import { readFileSync } from "node:fs";

const ROOT = new URL("../", import.meta.url);
const PRODUCT_NAMES = new Set(["browser", "cli", "lsp", "textlint", "vscode", "zed"]);

function fail(message) {
  throw new Error(message);
}

export function loadDistributionPlan() {
  return JSON.parse(readFileSync(new URL("release/distribution-plan.json", ROOT), "utf8"));
}

export function selectProduct(plan, product) {
  if (!PRODUCT_NAMES.has(product)) fail(`unsupported release product: ${product}`);
  if (plan.schemaVersion !== 3 || !Array.isArray(plan.products)) {
    fail("distribution plan schemaVersion must be 3");
  }
  const selected = plan.products.filter((entry) => entry.product === product);
  if (selected.length !== 1) fail(`distribution plan must contain one ${product} product`);
  return selected[0];
}

export function productVersion(productPlan) {
  const [relativePath, locator] = productPlan.versionSource.split("#", 2);
  if (!relativePath || !locator) fail(`invalid product version source: ${productPlan.versionSource}`);
  const source = readFileSync(new URL(relativePath, ROOT), "utf8");
  let version;
  if (relativePath.endsWith(".json")) {
    version = locator.split(".").reduce((value, key) => value?.[key], JSON.parse(source));
  } else if (locator === "package.version") {
    const packageSection = source.match(/^\[package\]\s*$([\s\S]*?)(?=^\[|(?![\s\S]))/m)?.[1];
    version = packageSection?.match(/^version\s*=\s*"([^"]+)"$/m)?.[1];
  } else if (locator === "version") {
    version = source.match(/^version\s*=\s*"([^"]+)"$/m)?.[1];
  }
  if (typeof version !== "string" || !/^\d+\.\d+\.\d+$/.test(version)) {
    fail(`invalid version in ${productPlan.versionSource}`);
  }
  return version;
}

export function plannedProductAssets(plan, productPlan, version) {
  if (productPlan.build === "cargo-dist") {
    return plan.targets.map((target) => ({
      archive: target.archive,
      executable: productPlan.executable.replace("{executableSuffix}", target.executableSuffix),
      kind: productPlan.assetKind,
      name: productPlan.assetName.replace("{target}", target.triple),
      target: target.triple,
    })).sort((left, right) => left.name.localeCompare(right.name));
  }
  return [{
    archive: productPlan.archive,
    executable: null,
    kind: productPlan.assetKind,
    name: productPlan.assetName.replace("{version}", version),
    target: null,
  }];
}

export function productIdentity(product) {
  if (!PRODUCT_NAMES.has(product)) fail(`unsupported release product: ${product}`);
  return product;
}
