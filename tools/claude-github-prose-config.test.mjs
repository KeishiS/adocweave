import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const settings = JSON.parse(
  readFileSync(new URL("../.claude/settings.json", import.meta.url), "utf8")
);

test("同期PreToolUse hookが投稿前検査を要求する", () => {
  const group = settings.hooks?.PreToolUse?.[0];
  assert.equal(group.matcher, "^Bash$");
  assert.equal(group.hooks.length, 1);
  const hook = group.hooks[0];
  assert.equal(hook.type, "command");
  assert.equal(hook.timeout, 10);
  assert.match(hook.command, /tools\/github-prose-hook\.mjs/);
});

test("permissionsが直接投稿を禁止して検査付き投稿を許可する", () => {
  for (const required of [
    "Bash(gh issue create:*)",
    "Bash(gh issue new:*)",
    "Bash(gh issue comment:*)",
    "Bash(gh pr create:*)",
    "Bash(gh pr new:*)",
    "Bash(gh pr comment:*)"
  ]) {
    assert.ok(
      settings.permissions?.deny?.includes(required),
      `permissions.denyに必要な規則がありません：${required}`
    );
  }
  assert.ok(settings.permissions?.allow?.includes("Bash(node tools/checked-gh-prose.mjs:*)"));
});

test("題名以外のedit操作をpermissionsで止めない", () => {
  for (const command of ["Bash(gh issue edit:*)", "Bash(gh pr edit:*)", "Bash(gh pr review:*)"]) {
    assert.ok(
      !settings.permissions?.deny?.includes(command),
      `hookが精密に判定するため、permissions.denyへ含めません：${command}`
    );
  }
});

test("skillが検査付きラッパーの使用を案内する", () => {
  const skill = readFileSync(
    new URL("../.claude/skills/github-prose/SKILL.md", import.meta.url),
    "utf8"
  );
  assert.match(skill, /^---\nname: github-prose\ndescription: \S/u);
  assert.match(skill, /tools\/checked-gh-prose\.mjs/);
});
