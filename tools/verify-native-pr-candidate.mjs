import { readdirSync } from "node:fs";
import { basename, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import {
  loadDistributionPlan,
  plannedProductAssets,
  productVersion,
  selectProduct,
} from "./product-release-plan.mjs";

export function expectedPullRequestAssets(plan, product) {
  const selected = selectProduct(plan, product);
  const assets = plannedProductAssets(plan, selected, productVersion(selected));
  if (selected.build !== "cargo-dist") return assets.map(({ name }) => name);
  const pullRequestTargets = new Set(
    plan.targets.filter(({ os }) => os === "darwin" || os === "win32").map(({ triple }) => triple),
  );
  return assets.filter(({ target }) => pullRequestTargets.has(target)).map(({ name }) => name).sort();
}

export function verifyPullRequestAssets(actual, plan, product) {
  const expected = expectedPullRequestAssets(plan, product);
  const sortedActual = [...actual].sort();
  if (JSON.stringify(sortedActual) !== JSON.stringify(expected)) {
    throw new Error(
      `pull request candidate mismatch:\nexpected: ${expected.join(", ")}\nactual: ${sortedActual.join(", ")}`,
    );
  }
}

function main() {
  const [product, candidateArgument] = process.argv.slice(2);
  if (!product || !candidateArgument) {
    process.stderr.write("usage: node tools/verify-native-pr-candidate.mjs PRODUCT CANDIDATE_DIRECTORY\n");
    process.exit(2);
  }
  const candidate = resolve(candidateArgument);
  const entries = readdirSync(candidate, { withFileTypes: true });
  if (entries.some((entry) => !entry.isFile())) {
    throw new Error("pull request candidate must contain files only");
  }
  verifyPullRequestAssets(
    entries.map(({ name }) => basename(name)),
    loadDistributionPlan(),
    product,
  );
  process.stdout.write(`pull request candidate verified: ${product}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main();
