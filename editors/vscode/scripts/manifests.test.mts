import assert from "node:assert/strict";
import test from "node:test";

import {
  type ExtensionManifest,
  readJson,
  supportedVSCodeFloor,
  validateVSCodeTypesCompatibility,
} from "./manifests.mts";

const manifest = readJson<ExtensionManifest>(new URL("../package.json", import.meta.url));

function withVersions(engine: string, types: string | undefined): ExtensionManifest {
  return {
    ...manifest,
    engines: { vscode: engine },
    devDependencies: types === undefined ? {} : { "@types/vscode": types },
  };
}

test("uses the declared minimum editor for integration tests", () => {
  assert.equal(supportedVSCodeFloor(withVersions("^1.138.0", "1.138.0")), "1.138.0");
});

test("accepts matching or older API types, including independent patch releases", () => {
  for (const types of ["1.138.0", "1.138.9", "1.107.0"]) {
    assert.doesNotThrow(() => validateVSCodeTypesCompatibility(withVersions("^1.138.0", types)));
  }
});

test("rejects API types newer than the minimum editor before VSIX packaging", () => {
  for (const [engine, types] of [
    ["^1.107.0", "1.138.0"],
    ["^1.138.0", "2.0.0"],
  ] as const) {
    assert.throws(
      () => validateVSCodeTypesCompatibility(withVersions(engine, types)),
      /requires a newer editor/,
    );
  }
});

test("rejects missing or unpinned API types", () => {
  for (const types of [undefined, "^1.138.0", "1.138"]) {
    assert.throws(
      () => validateVSCodeTypesCompatibility(withVersions("^1.138.0", types)),
      /must be pinned/,
    );
  }
});
