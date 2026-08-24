import assert from "node:assert/strict";
import test from "node:test";

import plan from "../release/distribution-plan.json" with { type: "json" };
import { candidatePlan } from "./product-candidate-plan.mjs";

test("選択したnative製品だけをcandidateへ割り当てる", () => {
  const result = candidatePlan("lsp", () => false, plan);
  assert.deepEqual(result.nativeCandidates.include, [{ artifact_key: "lsp", product: "lsp" }]);
  assert.equal(result.native.include.length, plan.targets.length);
  assert.ok(result.native.include.every(({ product }) => product === "lsp"));
  assert.deepEqual(result.scripts.include, []);
});

test("選択していない未公開製品を含めない", () => {
  const result = candidatePlan("cli", () => false, plan);
  assert.deepEqual(result.nativeCandidates.include, [{ artifact_key: "cli", product: "cli" }]);
  assert.equal(result.native.include.length, plan.targets.length);
  assert.ok(result.native.include.every(({ product }) => product === "cli"));
  assert.deepEqual(result.scripts.include, []);
});

test("選択したscript製品ではnative matrixを空にする", () => {
  const result = candidatePlan("wasm", () => false, plan);
  assert.deepEqual(result.nativeCandidates.include, []);
  assert.deepEqual(result.native.include, []);
  assert.deepEqual(result.scripts.include, [{ artifact_key: "wasm", product: "wasm" }]);
});

test("未知の製品を拒否する", () => {
  assert.throws(() => candidatePlan("unknown", () => false, plan), /一意に解決/);
});

test("選択した製品のtagが存在する場合は拒否する", () => {
  assert.throws(() => candidatePlan("lsp", () => true, plan), /すでに存在/);
});
