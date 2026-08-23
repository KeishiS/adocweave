import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { formatDiagnostic, lintGitHubMarkdown } from "./github-markdown-lint.mjs";

const REPOSITORY_ROOT = fileURLToPath(new URL("../../", import.meta.url));

// npmへ公開するpackageだけがREADME.mdを収録する。対象を一覧で持たず
// package.jsonから導くため、公開packageが増えても検査から漏れない。
export function listPublishedReadmes(repositoryRoot = REPOSITORY_ROOT) {
  const manifests = execFileSync("git", ["ls-files", "-z", "*package.json"], {
    cwd: repositoryRoot,
    encoding: "utf8"
  })
    .split("\0")
    .filter(Boolean);
  const readmes = [];
  for (const path of manifests) {
    const manifest = JSON.parse(readFileSync(resolve(repositoryRoot, path), "utf8"));
    if (manifest.private === true || !Array.isArray(manifest.files)) continue;
    if (!manifest.files.includes("README.md")) continue;
    readmes.push(join(dirname(path), "README.md").split("\\").join("/"));
  }
  return Object.freeze(readmes.sort());
}

async function main() {
  const readmes = listPublishedReadmes();
  if (readmes.length === 0) {
    throw new Error("公開packageのREADME.mdが見つかりません。");
  }
  const documents = readmes.map((path) => ({
    field: path,
    markdown: readFileSync(resolve(REPOSITORY_ROOT, path), "utf8")
  }));
  const diagnostics = await lintGitHubMarkdown(documents);
  for (const diagnostic of diagnostics) process.stderr.write(`${formatDiagnostic(diagnostic)}\n`);
  if (diagnostics.length > 0) process.exitCode = 1;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`公開packageのREADME検査に失敗しました：${error.message}\n`);
    process.exitCode = 2;
  });
}
