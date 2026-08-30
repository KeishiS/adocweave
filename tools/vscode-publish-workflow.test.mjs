import test from "node:test";
import { verifyVscodePublishWorkflow } from "./vscode-publish-workflow.mjs";

test("VS Code拡張のtag、候補および二つのレジストリ公開を検査する", () => {
  verifyVscodePublishWorkflow();
});
