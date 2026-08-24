import assert from "node:assert/strict";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";

import {
  PRODUCT_IDS,
  syncReleaseVersion,
  validateRegistry,
} from "./sync-release-version.mjs";

function write(directory, path, source) {
  mkdirSync(dirname(join(directory, path)), { recursive: true });
  writeFileSync(join(directory, path), source);
}

function product(id, overrides = {}) {
  return {
    id,
    authority: {
      type: "literal",
      path: `products/${id}.txt`,
      template: "version={version}",
      count: 1,
    },
    targets: [],
    generators: [],
    ...overrides,
  };
}

function registry() {
  return {
    schemaVersion: 2,
    products: PRODUCT_IDS.map((id) =>
      id === "cli"
        ? product(id, {
            targets: [
              {
                type: "literal",
                path: "component.txt",
                template: "component-version={version}",
                count: 1,
              },
              {
                type: "cargo-lock",
                path: "Cargo.lock",
                packages: ["adocweave-cli"],
              },
            ],
            generators: [
              { id: "protocol", outputs: [{ path: "generated/protocol.txt" }] },
            ],
          })
        : product(id),
    ),
  };
}

function fixture() {
  const directory = mkdtempSync(join(tmpdir(), "adocweave-version-sync."));
  for (const id of PRODUCT_IDS) write(directory, `products/${id}.txt`, "version=1.2.3\n");
  write(directory, "component.txt", "component-version=1.2.3\n");
  write(
    directory,
    "Cargo.lock",
    '[[package]]\nname = "adocweave-cli"\nversion = "1.2.3"\n\n[[package]]\nname = "dependency"\nversion = "1.3.0"\nsource = "registry+https://example.invalid/index"\n',
  );
  write(directory, "generated/protocol.txt", "protocol 1.2.3\n");
  write(directory, "unmanaged.txt", "old-version=0.10.0\n");
  return {
    directory,
    root: pathToFileURL(`${directory}/`),
    cleanup: () => rmSync(directory, { recursive: true, force: true }),
  };
}

function generatedRunner({ mode, root, current, version, generator }) {
  if (mode === "check") return;
  for (const output of generator.outputs) {
    const path = new URL(output.path, root);
    writeFileSync(path, readFileSync(path, "utf8").replaceAll(current, version));
  }
}

test("登録した製品と製品内のroutingだけを受理する", () => {
  assert.doesNotThrow(() => validateRegistry(registry()));
  // targetと生成物を持つ製品は限られるため、位置ではなく内容で選ぶ。
  const withTargets = (value) => value.products.find((entry) => entry.targets.length > 0);
  const withGenerators = (value) => value.products.find((entry) => entry.generators.length > 0);
  const cases = [
    (value) => value.products.pop(),
    (value) => value.products.push(structuredClone(value.products[0])),
    (value) => { value.products[0].id = "unknown"; },
    (value) => { delete value.products[0].authority.count; },
    (value) => { withTargets(value).targets[0].unknown = true; },
    (value) => { withGenerators(value).generators[0].id = "unknown"; },
    (value) => { withGenerators(value).generators[0].id = "public-conformance"; },
    (value) => { withGenerators(value).generators[0].outputs[0].path = "../outside"; },
  ];
  for (const mutate of cases) {
    const value = registry();
    mutate(value);
    assert.throws(() => validateRegistry(value));
  }
});

test("選択した製品の正本、targetおよび生成物だけを更新する", () => {
  const scope = fixture();
  try {
    const unmanaged = readFileSync(join(scope.directory, "unmanaged.txt"), "utf8");
    const result = syncReleaseVersion({
      root: scope.root,
      mode: "update",
      product: "cli",
      version: "1.3.0",
      registry: registry(),
      runGenerator: generatedRunner,
    });
    assert.deepEqual(result.changed.sort(), [
      "Cargo.lock",
      "component.txt",
      "generated/protocol.txt",
      "products/cli.txt",
    ]);
    assert.equal(readFileSync(join(scope.directory, "products/cli.txt"), "utf8"), "version=1.3.0\n");
    assert.match(readFileSync(join(scope.directory, "Cargo.lock"), "utf8"), /name = "dependency"\nversion = "1\.3\.0"/);
    assert.equal(readFileSync(join(scope.directory, "unmanaged.txt"), "utf8"), unmanaged);
    for (const id of PRODUCT_IDS.filter((id) => id !== "cli")) {
      assert.equal(readFileSync(join(scope.directory, `products/${id}.txt`), "utf8"), "version=1.2.3\n");
    }
    assert.doesNotThrow(() =>
      syncReleaseVersion({
        root: scope.root,
        mode: "check",
        product: "cli",
        registry: registry(),
        runGenerator: generatedRunner,
      }),
    );
  } finally {
    scope.cleanup();
  }
});

test("LSPだけの更新はBrowser、textlintおよびeditorをbyte単位で変更しない", () => {
  const scope = fixture();
  try {
    const actualRegistry = JSON.parse(
      readFileSync(new URL("../release/version-sync.json", import.meta.url), "utf8"),
    );
    write(scope.directory, "crates/adocweave-lsp/Cargo.toml", '[package]\nname = "adocweave-lsp"\nversion = "0.46.2"\n');
    write(scope.directory, "Cargo.lock", '[[package]]\nname = "adocweave-lsp"\nversion = "0.46.2"\n');
    const unchangedPaths = new Set(actualRegistry.products
      .filter(({ id }) => id !== "lsp")
      .flatMap(({ authority, targets, generators }) => [
        authority.path,
        ...targets.map(({ path }) => path),
        ...generators.flatMap(({ outputs }) => outputs.map(({ path }) => path)),
      ])
      .filter((path) => path !== "Cargo.lock"));
    for (const path of unchangedPaths) write(scope.directory, path, `sentinel:${path}\n`);
    const unchanged = new Map([...unchangedPaths]
      .map((path) => [path, readFileSync(join(scope.directory, path))]));

    const result = syncReleaseVersion({
      root: scope.root,
      mode: "update",
      product: "lsp",
      version: "0.46.3",
      registry: actualRegistry,
      runGenerator: generatedRunner,
    });

    assert.deepEqual(result.changed.sort(), ["Cargo.lock", "crates/adocweave-lsp/Cargo.toml"]);
    for (const [path, source] of unchanged) {
      assert.deepEqual(readFileSync(join(scope.directory, path)), source, path);
    }
  } finally {
    scope.cleanup();
  }
});

test("部分更新、古い更新先、未知製品および不足fileを変更前に拒否する", () => {
  const scope = fixture();
  try {
    const inventory = registry();
    write(scope.directory, "component.txt", "component-version=1.3.0\n");
    assert.throws(
      () => syncReleaseVersion({ root: scope.root, mode: "update", product: "cli", version: "1.3.0", registry: inventory, runGenerator: generatedRunner }),
      /version記録数/,
    );
    write(scope.directory, "component.txt", "component-version=1.2.3\n");
    assert.throws(
      () => syncReleaseVersion({ root: scope.root, mode: "update", product: "cli", version: "1.2.2", registry: inventory, runGenerator: generatedRunner }),
      /現在のversionより大きい/,
    );
    assert.throws(
      () => syncReleaseVersion({ root: scope.root, mode: "check", product: "unknown", registry: inventory, runGenerator: generatedRunner }),
      /未対応のproduct/,
    );
    rmSync(join(scope.directory, "component.txt"));
    assert.throws(
      () => syncReleaseVersion({ root: scope.root, mode: "check", product: "cli", registry: inventory, runGenerator: generatedRunner }),
      /管理対象fileがありません/,
    );
  } finally {
    scope.cleanup();
  }
});

test("失敗した生成処理の変更と未追跡fileを復元する", () => {
  const scope = fixture();
  try {
    const generated = join(scope.directory, "generated/protocol.txt");
    const before = readFileSync(generated, "utf8");
    const unexpected = join(scope.directory, "generated/unexpected.txt");
    assert.throws(
      () =>
        syncReleaseVersion({
          root: scope.root,
          mode: "update",
          product: "cli",
          version: "1.3.0",
          registry: registry(),
          runGenerator: () => {
            writeFileSync(generated, "changed\n");
            writeFileSync(unexpected, "unexpected\n");
            throw new Error("generator failure");
          },
        }),
      /generator failure/,
    );
    assert.equal(readFileSync(generated, "utf8"), before);
    assert.equal(existsSync(unexpected), false);
  } finally {
    scope.cleanup();
  }
});

test("stable SemVerだけを更新先として受理する", () => {
  for (const version of ["1.2", "v1.2.3", "1.2.3-rc.1", "01.2.3"]) {
    const scope = fixture();
    try {
      assert.throws(
        () => syncReleaseVersion({ root: scope.root, mode: "update", product: "cli", version, registry: registry(), runGenerator: generatedRunner }),
        /stable SemVer/,
      );
    } finally {
      scope.cleanup();
    }
  }
});
