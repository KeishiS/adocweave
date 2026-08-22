import assert from "node:assert/strict";
import test from "node:test";

import plan from "../release/distribution-plan.json" with { type: "json" };
import { candidatePlan } from "./product-candidate-plan.mjs";

test("tagがない製品だけを製品別candidateへ割り当てる", () => {
  const result = candidatePlan((tag) => tag !== "adocweave-lsp/v0.46.2", plan);
  assert.deepEqual(result.nativeCandidates.include, [{ artifact_key: "lsp", product: "lsp" }]);
  assert.equal(result.native.include.length, plan.targets.length);
  assert.ok(result.native.include.every(({ product }) => product === "lsp"));
  assert.deepEqual(result.scripts.include, []);
});

test("旧統一tagは製品tagの公開済み判定に使わない", () => {
  const result = candidatePlan((tag) => tag === "v0.46.2", plan);
  assert.deepEqual(result.nativeCandidates.include, [
    { artifact_key: "cli", product: "cli" },
    { artifact_key: "lsp", product: "lsp" },
  ]);
  assert.equal(result.native.include.length, plan.targets.length * 2);
  assert.equal(result.scripts.include.length, 4);
});

test("script製品だけが未公開の場合はnative candidate matrixを空にする", () => {
  const result = candidatePlan((tag) => tag !== "adocweave-browser/v0.46.2", plan);
  assert.deepEqual(result.nativeCandidates.include, []);
  assert.deepEqual(result.native.include, []);
  assert.deepEqual(result.scripts.include, [{ artifact_key: "browser", product: "browser" }]);
});
