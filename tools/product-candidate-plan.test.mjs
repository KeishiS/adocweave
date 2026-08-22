import assert from "node:assert/strict";
import test from "node:test";

import plan from "../release/distribution-plan.json" with { type: "json" };
import { candidatePlan } from "./product-candidate-plan.mjs";

test("tagがない製品だけを製品別candidateへ割り当てる", () => {
  const result = candidatePlan((tag) => tag !== "adocweave-lsp/v0.46.2", plan);
  assert.deepEqual(result.candidates.include, [{ artifact_key: "lsp", product: "lsp" }]);
  assert.equal(result.native.include.length, plan.targets.length);
  assert.ok(result.native.include.every(({ product }) => product === "lsp"));
  assert.deepEqual(result.scripts.include, []);
});

test("旧統一tagは製品tagの公開済み判定に使わない", () => {
  const result = candidatePlan((tag) => tag === "v0.46.2", plan);
  assert.equal(result.candidates.include.length, 6);
  assert.equal(result.native.include.length, plan.targets.length * 2);
  assert.equal(result.scripts.include.length, 4);
});
