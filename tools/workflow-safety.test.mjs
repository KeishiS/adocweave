import test from "node:test";
import { verifyWorkflowSafety } from "./workflow-safety.mjs";

test("共通のAction固定、権限およびCI gateを検査する", () => {
  verifyWorkflowSafety();
});
