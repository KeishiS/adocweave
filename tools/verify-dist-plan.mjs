import { readFileSync } from "node:fs";
import process from "node:process";

import { releaseFromTag, validateDistPlan } from "./release-contract.mjs";
import plan from "../release/distribution-plan.json" with { type: "json" };

const [path, tag] = process.argv.slice(2);
if (!path || !tag) {
  process.stderr.write("usage: node tools/verify-dist-plan.mjs PLAN_JSON TAG\n");
  process.exit(2);
}

try {
  const { product, productVersion } = releaseFromTag(tag);
  validateDistPlan(JSON.parse(readFileSync(path, "utf8")), plan, product, productVersion, tag);
  process.stdout.write(`dist plan verified: ${tag}\n`);
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
}
