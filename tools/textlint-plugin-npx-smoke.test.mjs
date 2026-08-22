import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  assertExpectedDiagnostic,
  npmInvocation,
  npxArguments,
  npxSettings,
  runTextlintPluginNpxSmoke,
} from "./textlint-plugin-npx-smoke.mjs";

const manifest = {
  name: "@adocweave/textlint-plugin-asciidoc",
  peerDependencies: { textlint: "15.8.0" },
};

test("指定したpackageを固定したnpx引数で検査する", async () => {
  const root = await mkdtemp(join(tmpdir(), "adocweave-textlint-npx-test-"));
  const archive = join(root, "plugin.tgz");
  let invocation;
  try {
    await writeFile(archive, "fixture");
    await runTextlintPluginNpxSmoke(archive, {
      manifest,
      invokeNpm: async (value) => {
        invocation = value;
        return {
          code: 1,
          stderr: "",
          stdout: JSON.stringify([{
            filePath: "document.adoc",
            messages: [{ line: 3, ruleId: "ja-technical-writing/sentence-length" }],
          }]),
        };
      },
    });
    assert.deepEqual(invocation.args, npxArguments(archive, npxSettings(manifest)));
    assert.deepEqual(invocation.args.slice(0, 7), [
      "exec",
      "--yes",
      "--package=textlint@15.8.0",
      `--package=${archive}`,
      "--package=textlint-rule-preset-ja-technical-writing@12.0.2",
      "--",
      "textlint",
    ]);
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

test("期待する診断がない成功らしい出力を拒否する", () => {
  assert.throws(
    () => assertExpectedDiagnostic(JSON.stringify([{ messages: [] }])),
    /expected sentence-length diagnostic/,
  );
});

test("単発実行設定の欠落を拒否する", () => {
  assert.throws(
    () => npxSettings(manifest, {}),
    /単発実行設定が不足/,
  );
});

test("WindowsではNode.jsからnpm CLIを起動する", () => {
  assert.deepEqual(
    npmInvocation({
      environment: {},
      executable: String.raw`C:\node\node.exe`,
      platform: "win32",
    }),
    {
      arguments: [String.raw`C:\node\node_modules\npm\bin\npm-cli.js`],
      command: String.raw`C:\node\node.exe`,
    },
  );
});
