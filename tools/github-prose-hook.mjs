import { fileURLToPath } from "node:url";

const gh = String.raw`(?:^|[\s;&|('])(?:\S*/)?gh`;
const directPosting = new RegExp(
  `${gh}\\s+(?:issue\\s+(?:create|new|comment)|pr\\s+(?:create|new|comment))(?:\\s|$)|` +
  `${gh}\\s+(?:issue|pr)\\s+(?:edit|review)\\b.*` +
  String.raw`(?:--title|-t|--body|-b|--body-file|-F)(?:[=\s]|$)`,
  "u"
);
const apiCommand = new RegExp(`${gh}\\s+api\\b`, "u");
const apiEndpoint = /(?:\/issues|\/pulls)(?:\/|\s|$)/u;
const apiProse = /(?:-f|--field|-F|--raw-field)\s+(?:title|body)=|--input(?:=|\s)/u;

export function inspectGitHubProse(input) {
  if (input?.hook_event_name !== "PreToolUse" || input?.tool_name !== "Bash") return undefined;
  const command = input.tool_input?.command;
  if (typeof command !== "string") return undefined;
  const normalized = command.replaceAll(/\\\r?\n/gu, " ");
  const directApiPosting = apiCommand.test(normalized) &&
    apiEndpoint.test(normalized) && apiProse.test(normalized);
  if (!directPosting.test(normalized) && !directApiPosting) return undefined;
  return {
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason:
        "GitHubへ文章を投稿するときは、リポジトリ直下の" +
        "tools/checked-gh-prose.mjsをghの代わりに実行してください。"
    }
  };
}

async function main() {
  const chunks = [];
  for await (const chunk of process.stdin) chunks.push(chunk);
  const result = inspectGitHubProse(JSON.parse(Buffer.concat(chunks).toString("utf8")));
  if (result !== undefined) process.stdout.write(`${JSON.stringify(result)}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`GitHub投稿前の禁止語検査に失敗しました：${error.message}\n`);
    process.exitCode = 2;
  });
}
