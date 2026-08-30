import process from "node:process";

import {
  loadWorkflowPolicyInputs,
  validateWasmPublication,
} from "./workflow-policy-library.mjs";

export function verifyWasmPublishWorkflow(inputs = loadWorkflowPolicyInputs()) {
  validateWasmPublication(inputs.workflows, inputs.wasmNpmSmoke);
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    verifyWasmPublishWorkflow();
    process.stdout.write("WebAssembly publish workflow verified\n");
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
