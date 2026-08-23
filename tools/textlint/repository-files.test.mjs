import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { listRepositoryAsciiDocFiles } from "./repository-files.mjs";

test("docs配下のAsciiDocを追跡状態にかかわらず列挙する", (context) => {
  const repositoryRoot = mkdtempSync(join(tmpdir(), "adocweave-textlint-"));
  context.after(() => rmSync(repositoryRoot, { recursive: true }));

  execFileSync("git", ["init", "--quiet"], { cwd: repositoryRoot });
  mkdirSync(join(repositoryRoot, "docs", "nested"), { recursive: true });
  writeFileSync(join(repositoryRoot, "README.adoc"), "= README\n");
  writeFileSync(join(repositoryRoot, "docs", "tracked.adoc"), "= 追跡済み\n");
  execFileSync("git", ["add", "README.adoc", "docs/tracked.adoc"], {
    cwd: repositoryRoot
  });

  writeFileSync(join(repositoryRoot, "docs", "nested", "untracked.adoc"), "= 未追跡\n");
  writeFileSync(join(repositoryRoot, "docs", "ignored.adoc"), "= 除外指定済み\n");
  writeFileSync(join(repositoryRoot, ".gitignore"), "docs/ignored.adoc\n");
  writeFileSync(join(repositoryRoot, "outside.adoc"), "= 対象外\n");

  assert.deepEqual(listRepositoryAsciiDocFiles(repositoryRoot), [
    "README.adoc",
    "docs/ignored.adoc",
    "docs/nested/untracked.adoc",
    "docs/tracked.adoc"
  ]);
});
