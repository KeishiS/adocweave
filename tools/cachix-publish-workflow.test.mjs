import test from "node:test";
import { verifyCachixPublishWorkflow } from "./cachix-publish-workflow.mjs";

test("Cachixへの送信とtokenなしの取得確認を検査する", () => {
  verifyCachixPublishWorkflow();
});
