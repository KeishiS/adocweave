import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const script = readFileSync(new URL("security-audit.sh", import.meta.url), "utf8");

test("Cargo advisoryはcargo-denyが最新databaseを使う一経路に集約する", () => {
  const commands = script.split("\n").filter((line) => line.startsWith("cargo deny "));
  assert.equal(commands.length, 2);
  for (const command of commands) {
    assert.match(command, /check --config deny\.toml --exclude-dev advisories licenses bans sources$/);
    assert.doesNotMatch(command, /--disable-fetch/);
  }
  assert.doesNotMatch(script, /cargo audit|rustsec-advisory-db-revision|--no-fetch/);
});

test("npm advisoryはVS Codeの配布runtime treeだけを監査する", () => {
  assert.equal(
    script.split("\n").filter((line) => line.startsWith("npm audit ")).join("\n"),
    "npm audit --omit=dev --prefix editors/vscode",
  );
  assert.doesNotMatch(script, /--include=dev|tools\/textlint/);
});

test("VS Code runtimeのlicenseと取得元を専用policyで検査する", () => {
  assert.match(script, /^node tools\/verify-vscode-dependencies\.mjs$/m);
});

test("advisory例外はdeny.toml内で理由、期限およびIssueを検査する", () => {
  assert.match(script, /yq -p toml -o json deny\.toml \| node tools\/verify-advisory-exceptions\.mjs/);
  assert.match(script, /tools\/verify-advisory-exceptions\.test\.mjs/);
});
