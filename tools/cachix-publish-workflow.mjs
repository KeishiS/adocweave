import process from "node:process";

import {
  loadWorkflowPolicyInputs,
  validateCachixPublication,
  validateExternalPublicationIsolation,
} from "./workflow-policy-library.mjs";

export function verifyCachixPublishWorkflow(inputs = loadWorkflowPolicyInputs()) {
  validateExternalPublicationIsolation(inputs.workflows);
  validateCachixPublication(inputs.workflows);
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    verifyCachixPublishWorkflow();
    process.stdout.write("Cachix publish workflow verified\n");
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
