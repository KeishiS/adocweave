import assert from "node:assert/strict";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";

import {
  checkNativeReleaseVersion,
  parseNativeVersionArguments,
  updateNativeReleaseVersion,
  workspaceVersion,
} from "./native-release-version.mjs";

test("一つのworkspace版をnative releaseのmanifestとlockfileで使用する", () => {
  assert.equal(checkNativeReleaseVersion(), workspaceVersion());
  assert.match(workspaceVersion(), /^\d+\.\d+\.\d+$/u);
});

test("native版の更新後も四つの正本だけで整合性を検査できる", (context) => {
  const rootPath = mkdtempSync(join(tmpdir(), "adocweave-native-version-"));
  context.after(() => rmSync(rootPath, { recursive: true, force: true }));
  mkdirSync(join(rootPath, "fuzz"));
  const current = workspaceVersion();
  const [major, minor] = current.split(".").map(Number);
  const next = `${major}.${minor + 1}.0`;
  const packages = [
    "adocweave-core",
    "adocweave",
    "adocweave-lsp",
    "adocweave-project",
    "adocweave-textlint",
    "adocweave-wasm",
  ];
  const lockfile = (names, version) => names
    .map((name) => `[[package]]\nname = "${name}"\nversion = "${version}"`)
    .join("\n\n");
  writeFileSync(
    join(rootPath, "Cargo.toml"),
    `[workspace]\nmembers = []\n\n[workspace.package]\nversion = "${current}"\n`,
  );
  writeFileSync(
    join(rootPath, "CHANGELOG.md"),
    `## [${current}]\n\n[${current}]: https://example.invalid/v${current}\n`,
  );
  writeFileSync(join(rootPath, "Cargo.lock"), lockfile(packages, current));
  writeFileSync(join(rootPath, "fuzz/Cargo.lock"), lockfile(["adocweave-core"], current));
  const root = pathToFileURL(`${rootPath}/`);
  const calls = [];
  const result = updateNativeReleaseVersion(next, {
    root,
    runCommand(args) {
      calls.push(args);
      const lockPath = args.includes("fuzz/Cargo.toml") ? "fuzz/Cargo.lock" : "Cargo.lock";
      const path = join(rootPath, lockPath);
      writeFileSync(path, readFileSync(path, "utf8").replaceAll(current, next));
    },
  });

  assert.deepEqual(result, { current, version: next });
  assert.equal(checkNativeReleaseVersion(root), next);
  assert.deepEqual(calls, [
    ["generate-lockfile"],
    ["generate-lockfile", "--manifest-path", "fuzz/Cargo.toml"],
  ]);
});

test("CLI引数はnative版の検査と更新だけを受理する", () => {
  assert.deepEqual(parseNativeVersionArguments(["--check"]), { mode: "check" });
  assert.deepEqual(parseNativeVersionArguments(["--version", "1.2.3"]), {
    mode: "update",
    version: "1.2.3",
  });
  for (const args of [[], ["--unknown", "value"], ["--version"], ["--check", "extra"]]) {
    assert.throws(() => parseNativeVersionArguments(args), /usage:/);
  }
});
