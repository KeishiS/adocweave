import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  checkReleaseVersion,
  parseReleaseVersionArguments,
  validateRegistry,
} from "./sync-release-version.mjs";
import { workspaceVersion } from "./release-version.mjs";

const ROOT = new URL("../", import.meta.url);
const registry = JSON.parse(readFileSync(new URL("release/version-sync.json", ROOT), "utf8"));

test("一つのworkspace版をnative releaseのmanifestとlockfileで使用する", () => {
  assert.equal(checkReleaseVersion({ registry }), workspaceVersion());
  assert.equal(workspaceVersion(), "0.56.0");
});

test("同期設定は製品別の正本を持たない", () => {
  assert.deepEqual(Object.keys(validateRegistry(registry)).sort(), [
    "cargoLocks",
    "literals",
    "schemaVersion",
  ]);
  assert.equal("products" in registry, false);
  assert.equal("authority" in registry, false);
  assert.equal(
    registry.literals.some(({ path }) => path === "packages/wasm/package.json"),
    false,
  );
  assert.equal(
    registry.literals.some(({ path }) =>
      path.startsWith("editors/vscode/") ||
      path === "packages/textlint-plugin-asciidoc/package.json"
    ),
    false,
  );
  assert.equal(
    registry.literals.some(({ path }) => path === "editors/zed/Cargo.toml"),
    false,
  );
  assert.equal(
    registry.literals.some(({ path }) => path === "editors/zed/extension.toml"),
    false,
  );
  assert.equal(
    registry.cargoLocks.some(({ path }) => path === "editors/zed/Cargo.lock"),
    false,
  );
});

test("CLI引数は一括検査と一括更新だけを受理する", () => {
  assert.deepEqual(parseReleaseVersionArguments(["--check"]), {
    mode: "check",
    version: undefined,
  });
  assert.deepEqual(parseReleaseVersionArguments(["--version", "1.2.3"]), {
    mode: "update",
    version: "1.2.3",
  });
  for (const args of [[], ["--product", "cli", "--check"], ["--version"], ["--check", "extra"]]) {
    assert.throws(() => parseReleaseVersionArguments(args), /使用方法/);
  }
});

test("不完全な同期設定を拒否する", () => {
  assert.throws(() => validateRegistry({ schemaVersion: 1 }), /不正/);
  assert.throws(
    () => validateRegistry({ schemaVersion: 1, literals: [{ path: "a" }], cargoLocks: [] }),
    /不正/,
  );
});
