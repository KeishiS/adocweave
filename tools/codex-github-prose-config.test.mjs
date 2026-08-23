import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

test("同期PreToolUse hookが投稿前検査を要求する", () => {
  const config = JSON.parse(readFileSync(new URL("../.codex/hooks.json", import.meta.url), "utf8"));
  const group = config.hooks?.PreToolUse?.[0];
  assert.equal(group.matcher, "^Bash$");
  assert.equal(group.hooks.length, 1);
  const hook = group.hooks[0];
  assert.equal(hook.type, "command");
  assert.equal(hook.async, undefined);
  assert.equal(hook.timeout, 10);
  assert.match(hook.command, /tools\/codex-github-prose-hook\.mjs/);
});
