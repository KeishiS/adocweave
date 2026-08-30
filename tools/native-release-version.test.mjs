import assert from "node:assert/strict";
import test from "node:test";

import {
  checkNativeReleaseVersion,
  parseNativeVersionArguments,
  workspaceVersion,
} from "./native-release-version.mjs";

test("一つのworkspace版をnative releaseのmanifestとlockfileで使用する", () => {
  assert.equal(checkNativeReleaseVersion(), workspaceVersion());
  assert.equal(workspaceVersion(), "0.56.0");
});

test("CLI引数はnative版の検査と更新だけを受理する", () => {
  assert.deepEqual(parseNativeVersionArguments(["--check"]), { mode: "check" });
  assert.deepEqual(parseNativeVersionArguments(["--version", "1.2.3"]), {
    mode: "update",
    version: "1.2.3",
  });
  for (const args of [[], ["--product", "cli", "--check"], ["--version"], ["--check", "extra"]]) {
    assert.throws(() => parseNativeVersionArguments(args), /usage:/);
  }
});
