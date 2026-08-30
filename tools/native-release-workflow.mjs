import process from "node:process";

import {
  loadWorkflowPolicyInputs,
  validateReleaseFlow,
  validateNativeVersionCommands,
} from "./workflow-policy-library.mjs";

export function verifyNativeReleaseWorkflow(inputs = loadWorkflowPolicyInputs()) {
  validateReleaseFlow(inputs.workflows, inputs.distConfiguration);
  validateNativeVersionCommands(inputs.releaseGuide);
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    verifyNativeReleaseWorkflow();
    process.stdout.write("native release workflow verified\n");
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
