import assert from "node:assert/strict";
import test from "node:test";

import { inspectGitHubProse } from "./github-prose-hook.mjs";

function inspect(command) {
  return inspectGitHubProse({
    hook_event_name: "PreToolUse", tool_name: "Bash", tool_input: { command }
  });
}

test("GitHubへ文章を投稿しないコマンドを通す", () => {
  assert.equal(inspect("gh issue view 1"), undefined);
  assert.equal(inspect("gh issue edit 1 --add-label bug"), undefined);
  assert.equal(inspect("gh pr review 1 --approve"), undefined);
});

test("直接の文章投稿を拒否する", () => {
  for (const command of [
    "gh issue create --title 題名 --body 本文",
    "gh issue comment 1 --body-file /tmp/body.md",
    "gh pr create --title 題名 --body 本文",
    "gh pr edit 1 --body=本文",
    "gh pr review 1 --comment --body 本文",
    "gh api repos/o/r/issues/1/comments -f body=本文",
    "gh pr edit 1 \\\n  --body 本文",
    "gh issue edit 1 \\\n  --title 題名",
    "gh api -f body=本文 repos/o/r/issues/1/comments",
    "gh api --input=payload.json repos/o/r/issues/1/comments",
    "/usr/bin/gh issue new --title 題名 --body 本文"
  ]) {
    assert.equal(inspect(command).hookSpecificOutput.permissionDecision, "deny");
  }
});

test("検査付きラッパーの呼び出しを通す", () => {
  assert.equal(
    inspect("node tools/checked-gh-prose.mjs issue comment 1 --body-file /tmp/body.md"),
    undefined
  );
});
