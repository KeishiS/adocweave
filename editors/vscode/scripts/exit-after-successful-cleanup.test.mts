import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import test from "node:test";

test("exits a successful verification process even when handles are still open", () => {
  const moduleUrl = new URL("./exit-after-successful-cleanup.mts", import.meta.url).href;
  const result = spawnSync(
    process.execPath,
    [
      "--input-type=module",
      "--eval",
      `
        import { exitAfterSuccessfulCleanup } from ${JSON.stringify(moduleUrl)};
        setInterval(() => {}, 60_000);
        exitAfterSuccessfulCleanup();
      `,
    ],
    {
      encoding: "utf8",
      timeout: 1_000,
    },
  );

  assert.equal(result.error, undefined);
  assert.equal(result.signal, null);
  assert.equal(result.status, 0);
});
