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

const NATIVE_PACKAGES = [
  "adocweave-core",
  "adocweave",
  "adocweave-lsp",
  "adocweave-project",
  "adocweave-textlint",
  "adocweave-wasm",
];

function lockfile(names, version) {
  return names
    .map((name) => `[[package]]\nname = "${name}"\nversion = "${version}"`)
    .join("\n\n");
}

function createFixture(context, { current, changelog }) {
  const rootPath = mkdtempSync(join(tmpdir(), "adocweave-native-version-"));
  context.after(() => rmSync(rootPath, { recursive: true, force: true }));
  mkdirSync(join(rootPath, "fuzz"));
  writeFileSync(
    join(rootPath, "Cargo.toml"),
    `[workspace]\nmembers = []\n\n[workspace.package]\nversion = "${current}"\n`,
  );
  writeFileSync(join(rootPath, "CHANGELOG.md"), changelog);
  writeFileSync(join(rootPath, "Cargo.lock"), lockfile(NATIVE_PACKAGES, current));
  writeFileSync(
    join(rootPath, "fuzz/Cargo.lock"),
    lockfile(["adocweave-core"], current),
  );
  return { root: pathToFileURL(`${rootPath}/`), rootPath };
}

test("一つのworkspace版をnative releaseのmanifestとlockfileで使用する", () => {
  assert.equal(checkNativeReleaseVersion(), workspaceVersion());
  assert.match(workspaceVersion(), /^\d+\.\d+\.\d+$/u);
});

test("用意したChangelogを変えずにnative版を更新する", (context) => {
  const current = workspaceVersion();
  const [major, minor] = current.split(".").map(Number);
  const next = `${major}.${minor + 1}.0`;
  const changelog = [
    `## [${next}] - 2026-09-01`,
    "",
    "- A new release.",
    "",
    `## [${current}] - 2026-08-31`,
    "",
    "- The previous release.",
    "",
    `[${next}]: https://example.invalid/compare/v${current}...v${next}`,
    `[${current}]: https://example.invalid/v${current}`,
    "",
  ].join("\n");
  const { root, rootPath } = createFixture(context, { current, changelog });
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
  assert.equal(readFileSync(join(rootPath, "CHANGELOG.md"), "utf8"), changelog);
  assert.deepEqual(calls, [
    ["generate-lockfile"],
    ["generate-lockfile", "--manifest-path", "fuzz/Cargo.toml"],
  ]);
});

test("新しいChangelog項目がなければファイルを変更しない", (context) => {
  const current = workspaceVersion();
  const [major, minor] = current.split(".").map(Number);
  const next = `${major}.${minor + 1}.0`;
  const changelog = [
    `## [${current}] - 2026-08-31`,
    "",
    `The next version [${next}] is not a heading or reference.`,
    `A second mention of [${next}] must not make it a release entry.`,
    "",
    `[${current}]: https://example.invalid/v${current}`,
    "",
  ].join("\n");
  const { root, rootPath } = createFixture(context, { current, changelog });
  const manifest = readFileSync(join(rootPath, "Cargo.toml"), "utf8");
  const lock = readFileSync(join(rootPath, "Cargo.lock"), "utf8");
  const fuzzLock = readFileSync(join(rootPath, "fuzz/Cargo.lock"), "utf8");
  let called = false;

  assert.throws(
    () => updateNativeReleaseVersion(next, {
      root,
      runCommand() {
        called = true;
      },
    }),
    new RegExp(`CHANGELOG.md must contain one dated heading for native version ${next}`, "u"),
  );
  assert.equal(called, false);
  assert.equal(readFileSync(join(rootPath, "Cargo.toml"), "utf8"), manifest);
  assert.equal(readFileSync(join(rootPath, "Cargo.lock"), "utf8"), lock);
  assert.equal(readFileSync(join(rootPath, "fuzz/Cargo.lock"), "utf8"), fuzzLock);
  assert.equal(readFileSync(join(rootPath, "CHANGELOG.md"), "utf8"), changelog);
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
