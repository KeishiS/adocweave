import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import process from "node:process";

import { loadDistributionPlan, productIdentity } from "./product-release.mjs";

export function candidatePlan(tagExists, plan = loadDistributionPlan()) {
  const products = plan.products
    .map(({ product }) => productIdentity(product, { plan }))
    .filter(({ tag }) => !tagExists(tag));
  const native = products.filter(({ entry }) => entry.build === "cargo-dist");
  const scripts = products.filter(({ entry }) => entry.build === "script");
  return {
    candidates: { include: products.map(({ product }) => ({ artifact_key: product, product })) },
    nativeCandidates: {
      include: native.map(({ product }) => ({ artifact_key: product, product })),
    },
    native: {
      include: native.flatMap(({ product, tag }) => plan.targets.map((target) => ({
        build: target.os === "win32" ? "windows" : "nix",
        nix: target.os === "linux",
        nixSystem: target.os === "linux" ? target.triple.replace("unknown-linux-musl", "linux") : "",
        product,
        runner: target.runner,
        tag,
        target: target.triple,
      }))),
    },
    scripts: { include: scripts.map(({ product }) => ({ artifact_key: product, product })) },
  };
}

function main(output) {
  if (!output) throw new Error("使用方法：node tools/product-candidate-plan.mjs GITHUB_OUTPUT");
  const result = candidatePlan((tag) => {
    try {
      execFileSync("git", ["show-ref", "--verify", "--quiet", `refs/tags/${tag}`]);
      return true;
    } catch {
      return false;
    }
  });
  appendFileSync(output, [
    `candidate_required=${result.candidates.include.length > 0}`,
    `native_required=${result.native.include.length > 0}`,
    `global_required=${result.scripts.include.length > 0}`,
    `preflight_required=${result.candidates.include.length > 0}`,
    `release_main=${result.candidates.include.length > 0}`,
    `candidate_matrix=${JSON.stringify(result.candidates)}`,
    `native_candidate_matrix=${JSON.stringify(result.nativeCandidates)}`,
    `native_matrix=${JSON.stringify(result.native)}`,
    `script_matrix=${JSON.stringify(result.scripts)}`,
    "",
  ].join("\n"));
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    main(process.argv[2]);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
