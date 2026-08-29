import assert from "node:assert/strict";
import { resolve } from "node:path";
import test from "node:test";

import {
  selectServer,
  type SelectionDependencies,
  type SelectionOptions,
} from "../src/server-selection.js";

const storageDirectory = resolve("storage");
const options: SelectionOptions = { storageDirectory };

function dependencies(overrides: Partial<SelectionDependencies> = {}): SelectionDependencies {
  return {
    findOnPath: async () => undefined,
    downloadServer: async () => {
      throw new Error("downloadServerを呼んではいけません");
    },
    log: () => undefined,
    ...overrides,
  };
}

test("明示した絶対pathをPATHより先に選択します", async () => {
  let pathSearches = 0;
  const configuredPath = resolve("explicit-adocweave-lsp");
  const selected = await selectServer(
    { configuredPath, storageDirectory },
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

test("相対設定pathは取得へ倒さず拒否します", async () => {
  await assert.rejects(
    selectServer({ configuredPath: "relative/adocweave-lsp", storageDirectory }, dependencies()),
    /configured-server-path-not-absolute/,
  );
});

test("PATH上の相対候補は採用せず、取得へ進みます", async () => {
  const downloaded = resolve("storage", "adocweave-lsp");
  assert.deepEqual(
    await selectServer(
      options,
      dependencies({
        findOnPath: async () => "relative/adocweave-lsp",
        downloadServer: async () => downloaded,
      }),
    ),
    { command: downloaded, source: "downloaded" },
  );
});

test("導入済みの実行ファイルがある間は自動取得を行いません", async () => {
  const configuredPath = resolve("explicit-adocweave-lsp");
  const pathCandidate = resolve("path-adocweave-lsp");
  const failing = dependencies({ findOnPath: async () => pathCandidate });

  assert.deepEqual(await selectServer({ configuredPath, storageDirectory }, failing), {
    command: configuredPath,
    source: "configured",
  });
  assert.deepEqual(await selectServer({ storageDirectory }, failing), {
    command: pathCandidate,
    source: "path",
  });
});

test("どちらにも見つからない場合だけ自動取得へ進みます", async () => {
  const downloaded = resolve("storage", "adocweave-lsp-0.47.0-target", "adocweave-lsp");
  let requested: string | undefined;

  assert.deepEqual(
    await selectServer(
      { storageDirectory },
      dependencies({
        downloadServer: async (directory) => {
          requested = directory;
          return downloaded;
        },
      }),
    ),
    { command: downloaded, source: "downloaded" },
  );
  assert.equal(requested, storageDirectory);
});
