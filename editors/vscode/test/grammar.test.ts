import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";

import { loadWASM, OnigScanner, OnigString } from "vscode-oniguruma";
import { Registry, type IGrammar } from "vscode-textmate";

interface ScopeFixture {
  readonly scope: string;
  readonly source: string;
}

async function loadGrammar(): Promise<IGrammar> {
  const wasm = await readFile(require.resolve("vscode-oniguruma/release/onig.wasm"));
  await loadWASM(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength));
  const grammarSource = await readFile(
    join(__dirname, "..", "syntaxes", "asciidoc.tmLanguage.json"),
    "utf8",
  );
  const registry = new Registry({
    loadGrammar: async (scopeName) =>
      scopeName === "text.asciidoc" ? JSON.parse(grammarSource) : null,
    onigLib: Promise.resolve({
      createOnigScanner: (sources) => new OnigScanner(sources),
      createOnigString: (source) => new OnigString(source),
    }),
  });
  const grammar = await registry.loadGrammar("text.asciidoc");
  assert.ok(grammar);
  return grammar;
}

interface SpanFixture {
  readonly source: string;
  readonly scope: string;
  /** scopeが付く範囲として期待する文字列。行全体へ漏れていないことを確かめる。 */
  readonly spans: readonly string[];
}

test("inline書式は日本語に隣接しても、本体解析器と同じ範囲で閉じます", async () => {
  // Onigurumaの\wはUnicodeの語文字に一致し、日本語の「の」も含む。CJKは文字の形で
  // 語を分けるため隣接するCJK文字は語境界とする、という本体のcrate::cjkの判断へ
  // 文法定義を揃える。#762
  const fixtures: readonly SpanFixture[] = [
    {
      source: "全オプション、標準入力、``include::``の扱いはxref:a.adoc[コマンドライン]を参照",
      scope: "markup.inline.raw.asciidoc",
      spans: ["``", "include::", "``"],
    },
    {
      source: "標準入力``include::``の扱い",
      scope: "markup.inline.raw.asciidoc",
      spans: ["``", "include::", "``"],
    },
    { source: "入力、*強調*の扱い", scope: "markup.bold.asciidoc", spans: ["*", "強調", "*"] },
    { source: "入力、_斜体_の扱い", scope: "markup.italic.asciidoc", spans: ["_", "斜体", "_"] },
    // 英語の既存の挙動は変えない。語の内側のマーカーは書式にならない。
    { source: "a `code` b", scope: "markup.inline.raw.asciidoc", spans: ["`", "code", "`"] },
    { source: "snake_case_name and x*y*z", scope: "markup.bold.asciidoc", spans: [] },
  ];
  const grammar = await loadGrammar();
  for (const fixture of fixtures) {
    const spans = grammar
      .tokenizeLine(fixture.source, null)
      .tokens.filter((token) => token.scopes.includes(fixture.scope))
      .map((token) => fixture.source.slice(token.startIndex, token.endIndex));
    assert.deepEqual(
      spans,
      fixture.spans,
      `${JSON.stringify(fixture.source)}の${fixture.scope}の範囲が期待と異なります`,
    );
  }
});

test("TextMate grammarは代表的なAsciiDoc字句へ安定したscopeを付与します", async () => {
  const fixtures = JSON.parse(
    await readFile(join(__dirname, "fixtures", "grammar-scopes.json"), "utf8"),
  ) as ScopeFixture[];
  const grammar = await loadGrammar();
  for (const fixture of fixtures) {
    const scopes = grammar
      .tokenizeLine(fixture.source, null)
      .tokens.flatMap((token) => token.scopes);
    assert.ok(
      scopes.includes(fixture.scope),
      `${JSON.stringify(fixture.source)}に${fixture.scope}がありません：${scopes.join(", ")}`,
    );
  }
});
