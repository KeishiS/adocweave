import test from "node:test";
import { verifyWasmPublishWorkflow } from "./wasm-publish-workflow.mjs";

test("WebAssemblyパッケージのtag、候補およびnpm公開を検査する", () => {
  verifyWasmPublishWorkflow();
});
