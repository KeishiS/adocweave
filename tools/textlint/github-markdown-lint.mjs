import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { TextlintKernel } from "@textlint/kernel";
import markdownPlugin from "@textlint/textlint-plugin-markdown";

import { createTerminologyRules } from "./terminology-rule.mjs";

export const terminologyCatalog = JSON.parse(
  readFileSync(new URL("../../config/japanese-terminology.json", import.meta.url), "utf8")
);
const markdown = markdownPlugin.default ?? markdownPlugin;

export async function lintGitHubMarkdown(documents, catalog = terminologyCatalog) {
  const kernel = new TextlintKernel();
  const rules = createTerminologyRules(catalog, { ignoreStandaloneUrls: true });
  const diagnostics = [];
  for (const document of documents) {
    if (typeof document?.field !== "string" || typeof document?.markdown !== "string") {
      throw new Error("検査対象にはfieldとmarkdownを文字列で指定してください。");
    }
    const result = await kernel.lintText(document.markdown, {
      ext: ".md",
      filePath: "github.md",
      plugins: [{ pluginId: "markdown", plugin: markdown }],
      rules
    });
    for (const message of result.messages) {
      diagnostics.push(Object.freeze({
        field: document.field,
        ruleId: message.ruleId,
        line: message.line,
        column: message.column,
        range: message.range,
        severity: message.severity,
        message: message.message
      }));
    }
  }
  return Object.freeze(diagnostics);
}

export function formatDiagnostic(diagnostic) {
  const severity = diagnostic.severity === 1
    ? "warning"
    : diagnostic.severity === 2 ? "error" : "info";
  return `${diagnostic.field}:${diagnostic.line}:${diagnostic.column}: ` +
    `${severity}: ${diagnostic.message} (${diagnostic.ruleId})`;
}

async function main() {
  const args = process.argv.slice(2);
  if (args.length === 0 || args.length % 2 !== 0) {
    throw new Error("使用方法：node tools/textlint/github-markdown-lint.mjs FIELD MARKDOWN_FILE [...]");
  }
  const documents = [];
  for (let index = 0; index < args.length; index += 2) {
    documents.push({ field: args[index], markdown: readFileSync(args[index + 1], "utf8") });
  }
  const diagnostics = await lintGitHubMarkdown(documents);
  for (const diagnostic of diagnostics) process.stderr.write(`${formatDiagnostic(diagnostic)}\n`);
  if (diagnostics.some(({ severity }) => severity === 2)) process.exitCode = 1;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`GitHub文章の禁止語検査に失敗しました：${error.message}\n`);
    process.exitCode = 2;
  });
}
