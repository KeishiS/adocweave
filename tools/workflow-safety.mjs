import process from "node:process";

import {
  loadWorkflowPolicyInputs,
  validateCiGates,
  validatePermissions,
  validatePinnedActions,
} from "./workflow-policy-library.mjs";

export function verifyWorkflowSafety(inputs = loadWorkflowPolicyInputs()) {
  validatePinnedActions(inputs.workflows);
  validatePermissions(inputs.workflows);
  validateCiGates(inputs.workflows);
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    verifyWorkflowSafety();
    process.stdout.write("workflow safety verified\n");
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
