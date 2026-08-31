import assert from "node:assert/strict";
import test from "node:test";

import { TextlintKernel } from "@textlint/kernel";
import commentsFilter from "textlint-filter-rule-comments";

import plugin from "./processor.mjs";
import { createTerminologyRules } from "./terminology-rule.mjs";

const catalog = {
  schemaVersion: 4,
  forbiddenTerms: [
    {
      term: "禁止語",
      message: "別の表現を検討してください。"
    }
  ],
  warningTerms: [
    {
      term: "版",
      message: "バージョンの意味か確認してください。"
    }
  ]
};

async function lint(source) {
  return new TextlintKernel().lintText(source, {
    ext: ".adoc",
    filePath: "test.adoc",
    plugins: [{ pluginId: "adocweave", plugin }],
    rules: createTerminologyRules(catalog),
    filterRules: [{ ruleId: "comments", rule: commentsFilter }]
  });
}

test("地の文にある禁止語と注意語の元位置および重大度を報告する", async () => {
  const result = await lint("= 文書\n\n😀禁止語と版です。\n");
  assert.equal(result.messages.length, 2);
  assert.equal(result.messages[0].line, 3);
  assert.equal(result.messages[0].column, 3);
  assert.equal(result.messages[0].severity, 2);
  assert.equal(result.messages[0].ruleId, "adocweave-terminology");
  assert.equal(result.messages[0].message, "別の表現を検討してください。");
  assert.equal(result.messages[1].line, 3);
  assert.equal(result.messages[1].column, 7);
  assert.equal(result.messages[1].severity, 1);
  assert.equal(result.messages[1].ruleId, "adocweave-terminology-warning");
  assert.equal(result.messages[1].message, "バージョンの意味か確認してください。");
});

test("inline codeとsource blockを検査しない", async () => {
  const result = await lint(
    "= 文書\n\n`禁止語` です。\n\n[source,text]\n----\n禁止語\n----\n"
  );
  assert.equal(result.messages.length, 0);
});

test("AsciiDocコメントによる局所的な抑制を適用する", async () => {
  const result = await lint(
    "= 文書\n\n// textlint-disable adocweave-terminology\n禁止語です。\n// textlint-enable adocweave-terminology\n\n禁止語です。\n"
  );
  assert.equal(result.messages.length, 1);
  assert.equal(result.messages[0].line, 7);
});

test("空のtermを規則生成時に拒否する", () => {
  const invalid = structuredClone(catalog);
  invalid.forbiddenTerms[0].term = "";
  assert.throws(() => createTerminologyRules(invalid), /termは空でない文字列/);
});

test("規則生成後のcatalog変更を検査へ反映しない", () => {
  const mutable = structuredClone(catalog);
  const rule = createTerminologyRules(mutable)[0].rule;
  mutable.forbiddenTerms[0].term = "変更後";
  mutable.forbiddenTerms[0].message = "変更されました。";
  mutable.forbiddenTerms.push({ ...catalog.forbiddenTerms[0], term: "追加語" });

  const reports = [];
  class RuleError extends Error {
    constructor(message, details) {
      super(message);
      this.details = details;
    }
  }
  const visitor = rule({
    Syntax: { Str: "Str" },
    RuleError,
    locator: { range: (range) => range },
    report: (_node, error) => reports.push(error)
  });
  visitor.Str({ value: "禁止語、変更後、追加語" });

  assert.equal(reports.length, 1);
  assert.equal(reports[0].message, "別の表現を検討してください。");
  assert.deepEqual(reports[0].details.padding, [0, 3]);
});
