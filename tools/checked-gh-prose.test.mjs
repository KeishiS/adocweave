import assert from "node:assert/strict";
import test from "node:test";

import { runCheckedGh } from "./checked-gh-prose.mjs";

const catalog = {
  schemaVersion: 4,
  forbiddenTerms: [{
    term: "禁止語",
    message: "推奨表現へ変更してください。"
  }],
  warningTerms: [{
    term: "版",
    message: "バージョンの意味か確認してください。"
  }]
};

test("検査済みの題名と本文からgh引数を組み直す", async () => {
  const calls = [];
  const status = await runCheckedGh(
    ["pr", "create", "--title", "題名です", "--body-file", "/tmp/body.md"],
    {
      catalog,
      readText: () => "本文です。",
      execute: (...args) => { calls.push(args); return { status: 0 }; }
    }
  );
  assert.equal(status, 0);
  assert.deepEqual(calls[0][1], [
    "pr", "create", "--title", "題名です", "--body", "本文です。"
  ]);
});

test("禁止語があればghを実行しない", async () => {
  let executed = false;
  const status = await runCheckedGh(
    ["issue", "comment", "683", "--body", "禁止語です。"],
    { catalog, execute: () => { executed = true; return { status: 0 }; }, report: () => {} }
  );
  assert.equal(status, 1);
  assert.equal(executed, false);
});

test("注意語だけなら警告を報告してghを実行する", async () => {
  const reports = [];
  let executed = false;
  const status = await runCheckedGh(
    ["issue", "comment", "683", "--body", "安定版です。"],
    {
      catalog,
      execute: () => { executed = true; return { status: 0 }; },
      report: (diagnostic) => reports.push(diagnostic)
    }
  );
  assert.equal(status, 0);
  assert.equal(executed, true);
  assert.equal(reports.length, 1);
  assert.equal(reports[0].severity, 1);
});

test("検査対象のない操作とstdinの本文を拒否する", async () => {
  await assert.rejects(runCheckedGh(["pr", "review", "1", "--approve"], { catalog }), /検査対象/);
  await assert.rejects(
    runCheckedGh(["issue", "comment", "1", "--body-file", "-"], { catalog }),
    /実ファイル/
  );
});

test("短縮形、重複および対話的なoptionを拒否する", async () => {
  for (const args of [
    ["issue", "edit", "1", "--body", "本文です。", "-b別本文です。"],
    ["issue", "edit", "1", "--body", "本文です。", "--body=別本文です。"],
    ["pr", "create", "--title", "題名です", "--body", "本文です。", "--web"]
  ]) {
    await assert.rejects(runCheckedGh(args, { catalog }), /使用してください|重複|併用/);
  }
});

test("ghの終了状態と起動失敗を伝える", async () => {
  const args = ["issue", "comment", "1", "--body", "本文です。"];
  assert.equal(await runCheckedGh(args, { catalog, execute: () => ({ status: 7 }) }), 7);
  await assert.rejects(
    runCheckedGh(args, { catalog, execute: () => ({ error: new Error("起動失敗") }) }),
    /起動失敗/
  );
});
