import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { listPublishedReadmes } from "./package-readme-lint.mjs";

const repositoryRoot = fileURLToPath(new URL("../../", import.meta.url));
const readmes = listPublishedReadmes(repositoryRoot);

test("公開packageのREADMEを検査対象へ含める", () => {
  assert.ok(readmes.includes("packages/textlint-plugin-asciidoc/README.md"));
  for (const path of readmes) {
    assert.equal(existsSync(new URL(`../../${path}`, import.meta.url)), true);
  }
});

test("公開しないpackageのREADMEを検査対象へ含めない", () => {
  for (const path of readmes) {
    const manifest = JSON.parse(
      readFileSync(new URL(`../../${path.replace(/README\.md$/u, "package.json")}`, import.meta.url), "utf8")
    );
    assert.equal(manifest.private, undefined);
    assert.ok(manifest.files.includes("README.md"));
  }
  assert.equal(readmes.includes("tools/textlint/README.md"), false);
  assert.equal(readmes.includes("editors/vscode/README.md"), false);
});
