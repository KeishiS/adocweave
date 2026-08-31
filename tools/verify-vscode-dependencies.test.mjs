import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { validateVscodeRuntimeDependencies, vscodeRuntimePackages } from "./verify-vscode-dependencies.mjs";

const manifest = { private: true, version: "1.2.3" };
const lock = {
  lockfileVersion: 3,
  packages: {
    "": { version: "1.2.3" },
    "node_modules/runtime": {
      version: "1.0.0", license: "MIT",
      resolved: "https://registry.npmjs.org/runtime/-/runtime-1.0.0.tgz",
    },
    "node_modules/runtime/node_modules/transitive": {
      version: "2.0.0", license: "ISC",
      resolved: "https://registry.npmjs.org/transitive/-/transitive-2.0.0.tgz",
    },
    "node_modules/build-only": {
      dev: true, version: "3.0.0", license: "GPL-3.0-only",
      resolved: "https://example.invalid/build-only.tgz",
    },
  },
};

test("VS Codeの配布runtime treeだけを推移依存まで取得する", () => {
  assert.deepEqual(
    vscodeRuntimePackages(manifest, lock).map(({ name, version, license }) => ({ name, version, license })),
    [
      { name: "runtime", version: "1.0.0", license: "MIT" },
      { name: "transitive", version: "2.0.0", license: "ISC" },
    ],
  );
  assert.equal(validateVscodeRuntimeDependencies(manifest, lock).length, 2);
});

test("VS Code runtime dependencyの取得元とlicenseを制限する", () => {
  const changed = (entry) => ({
    ...lock,
    packages: {
      ...lock.packages,
      "node_modules/runtime": { ...lock.packages["node_modules/runtime"], ...entry },
    },
  });
  assert.throws(
    () => validateVscodeRuntimeDependencies(manifest, changed({ resolved: "https://example.invalid/runtime.tgz" })),
    /npm registry/,
  );
  assert.throws(
    () => validateVscodeRuntimeDependencies(manifest, changed({ license: "GPL-3.0-only" })),
    /license/,
  );
});

test("repositoryのVS Code runtime dependencyは配布方針を満たす", () => {
  const repositoryManifest = JSON.parse(readFileSync(new URL("../editors/vscode/package.json", import.meta.url)));
  const repositoryLock = JSON.parse(readFileSync(new URL("../editors/vscode/package-lock.json", import.meta.url)));
  // vscode-languageclientの推移依存9件と、取得したarchiveの展開に使うfflateです。
  assert.equal(validateVscodeRuntimeDependencies(repositoryManifest, repositoryLock).length, 10);
});
