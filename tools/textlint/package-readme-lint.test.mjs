import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { listPublishedReadmes } from "./package-readme-lint.mjs";

function writePackage(root, name, manifest) {
  const directory = join(root, "packages", name);
  mkdirSync(directory, { recursive: true });
  writeFileSync(join(directory, "package.json"), JSON.stringify(manifest));
  writeFileSync(join(directory, "README.md"), `# ${name}\n`);
}

test("追跡中の公開packageが収録するREADMEだけを列挙する", (context) => {
  const root = mkdtempSync(join(tmpdir(), "adocweave-package-readmes-"));
  context.after(() => rmSync(root, { force: true, recursive: true }));
  execFileSync("git", ["init", "--quiet"], { cwd: root });

  writePackage(root, "public", {
    name: "public",
    files: ["README.md", "index.mjs"],
  });
  writePackage(root, "private", {
    name: "private",
    private: true,
    files: ["README.md"],
  });
  writePackage(root, "without-readme", {
    name: "without-readme",
    files: ["index.mjs"],
  });
  execFileSync("git", ["add", "packages"], { cwd: root });

  writePackage(root, "untracked", {
    name: "untracked",
    files: ["README.md"],
  });

  assert.deepEqual(listPublishedReadmes(root), ["packages/public/README.md"]);
});
