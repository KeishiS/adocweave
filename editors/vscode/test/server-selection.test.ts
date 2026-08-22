import assert from "node:assert/strict";
import { resolve } from "node:path";
import test from "node:test";

import {
  selectServer,
  type SelectionDependencies,
  type SelectionOptions,
} from "../src/server-selection.js";

const options: SelectionOptions = {};

function dependencies(overrides: Partial<SelectionDependencies> = {}): SelectionDependencies {
  return {
    findOnPath: async () => undefined,
    ...overrides,
  };
}

test("明示した絶対pathをPATHより先に選択します", async () => {
  let pathSearches = 0;
  const configuredPath = resolve("explicit-adocweave-lsp");
  const selected = await selectServer(
    { configuredPath },
    dependencies({
      findOnPath: async () => {
        pathSearches += 1;
        return resolve("path-adocweave-lsp");
      },
    }),
  );
  assert.deepEqual(selected, { command: configuredPath, source: "configured" });
  assert.equal(pathSearches, 0);
});

test("設定がない場合はPATH上の絶対pathを選択します", async () => {
  const pathCandidate = resolve("path-adocweave-lsp");
  assert.deepEqual(
    await selectServer(options, dependencies({ findOnPath: async () => pathCandidate })),
    { command: pathCandidate, source: "path" },
  );
});

test("相対設定pathと候補不在を区別して拒否します", async () => {
  await assert.rejects(
    selectServer({ configuredPath: "relative/adocweave-lsp" }, dependencies()),
    /configured-server-path-not-absolute/,
  );
  await assert.rejects(selectServer(options, dependencies()), /language-server-not-found/);
  await assert.rejects(
    selectServer(options, dependencies({ findOnPath: async () => "relative/adocweave-lsp" })),
    /language-server-not-found/,
  );
});
