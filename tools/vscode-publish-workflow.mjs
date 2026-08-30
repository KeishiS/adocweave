import process from "node:process";

import {
  loadWorkflowPolicyInputs,
  validateVscodePublication,
} from "./workflow-policy-library.mjs";

export function verifyVscodePublishWorkflow(inputs = loadWorkflowPolicyInputs()) {
  validateVscodePublication(inputs.workflows);
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    verifyVscodePublishWorkflow();
    process.stdout.write("VS Code publish workflow verified\n");
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
