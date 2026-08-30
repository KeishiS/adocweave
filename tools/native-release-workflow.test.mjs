import test from "node:test";
import { verifyNativeReleaseWorkflow } from "./native-release-workflow.mjs";

test("native Releaseのtag、成果物およびCachix呼出しを検査する", () => {
  verifyNativeReleaseWorkflow();
});
