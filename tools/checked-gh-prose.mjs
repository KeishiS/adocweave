import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import {
  formatDiagnostic,
  lintGitHubMarkdown,
  terminologyCatalog
} from "./textlint/github-markdown-lint.mjs";

const supported = new Set([
  "issue create", "issue new", "issue edit", "issue comment",
  "pr create", "pr new", "pr edit", "pr comment", "pr review"
]);
const valueOptions = new Map([
  ["--title", "title"], ["--body", "body"], ["--body-file", "body-file"]
]);
const interactiveOptions = new Set([
  "--editor", "--web", "--recover", "--template",
  "--fill", "--fill-first", "--fill-verbose"
]);

function checkedArguments(args, readText) {
  const command = args.slice(0, 2).join(" ");
  if (!supported.has(command)) {
    throw new Error("対応する操作はissueまたはprのcreate、edit、comment、reviewです。");
  }
  const documents = [];
  const seen = new Set();
  const ghArgs = args.slice(0, 2);
  for (let index = 2; index < args.length; index += 1) {
    let option = args[index];
    let value;
    const equals = option.indexOf("=");
    if (equals !== -1) {
      value = option.slice(equals + 1);
      option = option.slice(0, equals);
    }
    if (option === "--" || interactiveOptions.has(option)) {
      throw new Error(`${option}は投稿前検査と併用できません。`);
    }
    if (/^-[tbF]/u.test(option)) {
      throw new Error("題名と本文には--title、--bodyまたは--body-fileを使用してください。");
    }
    const field = valueOptions.get(option);
    if (field === undefined) {
      ghArgs.push(args[index]);
      continue;
    }
    if (value === undefined) {
      index += 1;
      if (index >= args.length) throw new Error(`${option}に値を指定してください。`);
      value = args[index];
    }
    const normalizedField = field === "body-file" ? "body" : field;
    if (seen.has(normalizedField)) throw new Error(`${normalizedField}を重複して指定できません。`);
    seen.add(normalizedField);
    if (field === "body-file") {
      if (value === "-") throw new Error("--body-fileには実ファイルを指定してください。");
      value = readText(value, "utf8");
    }
    documents.push({ field: normalizedField, markdown: value });
  }
  if (documents.length === 0) {
    throw new Error("--title、--bodyまたは--body-fileで検査対象を明示してください。");
  }
  if ((command.endsWith("create") || command.endsWith("new")) &&
      (!seen.has("title") || !seen.has("body"))) {
    throw new Error("createでは--titleと--bodyまたは--body-fileを明示してください。");
  }
  const byField = new Map(documents.map((document) => [document.field, document.markdown]));
  if (byField.has("title")) ghArgs.push("--title", byField.get("title"));
  if (byField.has("body")) ghArgs.push("--body", byField.get("body"));
  return { documents, ghArgs };
}

export async function runCheckedGh(
  args,
  {
    catalog = terminologyCatalog,
    readText = readFileSync,
    execute = spawnSync,
    report = (diagnostic) => process.stderr.write(`${formatDiagnostic(diagnostic)}\n`)
  } = {}
) {
  const checked = checkedArguments(args, readText);
  const diagnostics = await lintGitHubMarkdown(checked.documents, catalog);
  for (const diagnostic of diagnostics) report(diagnostic);
  if (diagnostics.length > 0) return 1;
  const result = execute("gh", checked.ghArgs, { stdio: "inherit" });
  if (result.error) throw result.error;
  return result.status ?? 1;
}

async function main() {
  process.exitCode = await runCheckedGh(process.argv.slice(2));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`GitHubへの投稿前検査に失敗しました：${error.message}\n`);
    process.exitCode = 2;
  });
}
