import assert from "node:assert/strict";
import { test } from "node:test";

import { verifyBinaryCachePackage } from "./binary-cache-smoke.mjs";

test("Cachixから取得した実行ファイルのversionとLanguage Serverを検査する", async () => {
  const calls = [];
  let disposed = false;
  const deadline = { dispose: () => { disposed = true; } };
  await verifyBinaryCachePackage("/nix/store/example/bin/adocweave", "1.2.3", {
    createDeadline: () => deadline,
    execute(binary, arguments_, options) {
      calls.push(["execute", binary, arguments_, options]);
      return JSON.stringify({ packageVersion: "1.2.3" });
    },
    async runLsp(binary, arguments_, version, actualDeadline) {
      calls.push(["lsp", binary, arguments_, version, actualDeadline]);
    },
  });

  assert.deepEqual(calls, [
    [
      "execute",
      "/nix/store/example/bin/adocweave",
      ["--version", "--json"],
      { encoding: "utf8" },
    ],
    ["lsp", "/nix/store/example/bin/adocweave", ["lsp"], "1.2.3", deadline],
  ]);
  assert.equal(disposed, true);
});

test("workspace版と異なる実行ファイルを拒否する", async () => {
  await assert.rejects(
    verifyBinaryCachePackage("adocweave", "1.2.3", {
      execute: () => JSON.stringify({ packageVersion: "9.9.9" }),
    }),
    /version does not match/u,
  );
});

test("Language Server検査に失敗しても期限管理を終了する", async () => {
  let disposed = false;
  await assert.rejects(
    verifyBinaryCachePackage("adocweave", "1.2.3", {
      createDeadline: () => ({ dispose: () => { disposed = true; } }),
      execute: () => JSON.stringify({ packageVersion: "1.2.3" }),
      runLsp: async () => { throw new Error("LSP failed"); },
    }),
    /LSP failed/u,
  );
  assert.equal(disposed, true);
});
