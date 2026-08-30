import test from "node:test";
import { verifyTextlintPluginPublishWorkflow } from "./textlint-plugin-publish-workflow.mjs";

test("textlint用Processorのtag、候補およびnpm公開を検査する", () => {
  verifyTextlintPluginPublishWorkflow();
});
