import assert from "node:assert/strict";
import test from "node:test";

import { formatDiagnostic, lintGitHubMarkdown } from "./github-markdown-lint.mjs";

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

async function lint(markdown, field = "body") {
  return lintGitHubMarkdown([{ field, markdown }], catalog);
}

test("Markdownの文章とリンク表示文を検査する", async () => {
  const diagnostics = await lint([
    "# 禁止語の見出し", "", "本文の禁止語です。", "", "- 禁止語の項目", "",
    "> 禁止語の引用", "", "[禁止語の説明](https://example.test/禁止語)"
  ].join("\n"));
  assert.deepEqual(diagnostics.map(({ line, column }) => [line, column]), [
    [1, 3], [3, 4], [5, 3], [7, 3], [9, 2]
  ]);
});

test("codeとURLを検査しない", async () => {
  const diagnostics = await lint([
    "```text", "禁止語", "```", "", "``禁止語`` と `禁止語`", "",
    "https://example.test/禁止語 <https://example.test/禁止語>", "",
    "[説明](https://example.test/禁止語)"
  ].join("\n"));
  assert.equal(diagnostics.length, 0);
});

test("Unicodeを含む位置、複数の一致および重大度を報告する", async () => {
  const diagnostics = await lint("😀禁止語と禁止語、版です。", "title");
  assert.deepEqual(diagnostics.map(({ line, column, range, severity }) => ({
    line, column, range, severity
  })), [
    { line: 1, column: 3, range: [2, 5], severity: 2 },
    { line: 1, column: 7, range: [6, 9], severity: 2 },
    { line: 1, column: 11, range: [10, 11], severity: 1 }
  ]);
  assert.match(formatDiagnostic(diagnostics[0]), /^title:1:3: error: 推奨表現/u);
  assert.match(formatDiagnostic(diagnostics[2]), /^title:1:11: warning: バージョン/u);
});

test("空の文章を成功として扱う", async () => {
  assert.deepEqual(await lint(""), []);
});

test("不正な検査対象を拒否する", async () => {
  await assert.rejects(lintGitHubMarkdown([{}], catalog), /fieldとmarkdown/);
});
