import process from "node:process";

import {
  loadWorkflowPolicyInputs,
  validateTextlintPluginPublication,
} from "./workflow-policy-library.mjs";

export function verifyTextlintPluginPublishWorkflow(inputs = loadWorkflowPolicyInputs()) {
  validateTextlintPluginPublication(inputs.workflows, inputs.textlintNpmSmoke);
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    verifyTextlintPluginPublishWorkflow();
    process.stdout.write("textlint plugin publish workflow verified\n");
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
