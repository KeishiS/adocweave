import assert from "node:assert/strict";
import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { findOnPath } from "../src/path-search.js";

test("Windowsではshell scriptを候補にせずexeだけを選択します", async () => {
  const directory = await mkdtemp(join(tmpdir(), "adocweave-path-"));
  try {
    await writeFile(join(directory, "adocweave.cmd"), "@exit /b 0\r\n");
    assert.equal(await findOnPath("adocweave", directory, "win32"), undefined);
    await writeFile(join(directory, "adocweave.exe"), "fixture");
    assert.equal(
      await findOnPath("adocweave", directory, "win32"),
      join(directory, "adocweave.exe"),
    );
  } finally {
    await rm(directory, { force: true, recursive: true });
  }
});

test("Unixでは実行権限のあるfileだけを選択します", {
  skip: process.platform === "win32",
}, async () => {
  const directory = await mkdtemp(join(tmpdir(), "adocweave-path-"));
  const executable = join(directory, "adocweave");
  try {
    await writeFile(executable, "fixture");
    await chmod(executable, 0o644);
    assert.equal(await findOnPath("adocweave", directory, "linux"), undefined);
    await chmod(executable, 0o755);
    assert.equal(await findOnPath("adocweave", directory, "linux"), executable);
  } finally {
    await rm(directory, { force: true, recursive: true });
  }
});

test("PATH内の相対directoryを候補にしません", async () => {
  assert.equal(await findOnPath("adocweave", "relative-directory", process.platform), undefined);
});
