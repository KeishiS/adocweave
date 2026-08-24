import assert from "node:assert/strict";
import test from "node:test";

import {
  MAX_WASM_ARCHIVE_BYTES,
  MAX_WASM_MODULE_BYTES,
  assertWasmArtifactSizes,
  wasmArtifactSizeError,
} from "./wasm-release-budget.mjs";

test("WebAssembly performance budgets remain explicit release constants", () => {
  assert.equal(MAX_WASM_ARCHIVE_BYTES, 2_097_152);
  assert.equal(MAX_WASM_MODULE_BYTES, 1_310_720);
});

test("archive and raw WASM accept their exact performance budgets", () => {
  assert.equal(
    wasmArtifactSizeError(MAX_WASM_ARCHIVE_BYTES, MAX_WASM_MODULE_BYTES),
    null,
  );
});

test("archive rejects the first byte beyond its performance budget", () => {
  assert.equal(
    wasmArtifactSizeError(MAX_WASM_ARCHIVE_BYTES + 1, MAX_WASM_MODULE_BYTES),
    `archive exceeds 2 MiB: ${MAX_WASM_ARCHIVE_BYTES + 1}`,
  );
});

test("raw WASM rejects the first byte beyond its performance budget", () => {
  assert.equal(
    wasmArtifactSizeError(MAX_WASM_ARCHIVE_BYTES, MAX_WASM_MODULE_BYTES + 1),
    `WASM exceeds 1.25 MiB: ${MAX_WASM_MODULE_BYTES + 1}`,
  );
});

test("budget errors become release gate failures", () => {
  assert.throws(
    () => assertWasmArtifactSizes(MAX_WASM_ARCHIVE_BYTES, MAX_WASM_MODULE_BYTES + 1),
    /WASM exceeds 1\.25 MiB/,
  );
});
