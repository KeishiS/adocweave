import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";

import {
  syncReleaseVersion,
  validateRegistry,
} from "./sync-release-version.mjs";

function registry() {
  return {
    schemaVersion: 1,
    authority: {
      type: "literal",
      path: "release-manifest.json",
      template: "\"packageVersion\": \"{version}\"",
      count: 1,
    },
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
        packages: ["adocweave"],
      },
    ],
    generators: [
      { id: "protocol", outputs: [{ path: "generated/protocol.txt" }] },
      { id: "public-conformance", outputs: [{ path: "generated/public.txt" }] },
    ],
  };
}

function fixture() {
  const directory = mkdtempSync(join(tmpdir(), "adocweave-version-sync."));
  mkdirSync(join(directory, "generated"), { recursive: true });
  mkdirSync(join(directory, "fixtures"), { recursive: true });
  writeFileSync(
    join(directory, "release-manifest.json"),
    '{\n  "packageVersion": "1.2.3"\n}\n',
  );
  writeFileSync(join(directory, "component.txt"), "component-version=1.2.3\n");
  writeFileSync(
    join(directory, "Cargo.lock"),
    '[[package]]\nname = "adocweave"\nversion = "1.2.3"\n\n[[package]]\nname = "dependency"\nversion = "0.10.0"\nsource = "registry+https://example.invalid/index"\n',
  );
  writeFileSync(join(directory, "generated/protocol.txt"), "protocol 1.2.3\n");
  writeFileSync(join(directory, "generated/public.txt"), "public 1.2.3\n");
  writeFileSync(
    join(directory, "fixtures/old-version.json"),
    '{\n  "oldVersion": "0.10.0"\n}\n',
  );
  writeFileSync(
    join(directory, "fixtures/unrelated-version.txt"),
    "dependency-version=1.3.0\n",
  );
  return {
    directory,
    root: pathToFileURL(`${directory}/`),
    cleanup: () => rmSync(directory, { recursive: true, force: true }),
  };
}

function generatedRunner({ id, mode, root, current, version, generator }) {
  if (mode === "check") return;
  assert.ok(id === "protocol" || id === "public-conformance");
  for (const output of generator.outputs) {
    const path = new URL(output.path, root);
    const source = readFileSync(path, "utf8");
    writeFileSync(path, source.replaceAll(current, version));
  }
}

test("countがallの対象は件数を数えずすべてのversion記録を置き換える", () => {
  // 散文では版表記の数が文面ごとに変わります。件数を宣言すると、記録を1つ足すたびに
  // registryの数値を手で直す必要があり、置換漏れではなく数え直しの手間だけが増えます。
  const scope = fixture();
  try {
    writeFileSync(join(scope.directory, "notes.md"), "# v1.2.3\n\n1.2.3へ更新します。\n");
    const inventory = registry();
    inventory.targets.push({
      type: "literal",
      path: "notes.md",
      template: "{version}",
      count: "all",
    });
    assert.doesNotThrow(() => validateRegistry(inventory));
    syncReleaseVersion({
      root: scope.root,
      mode: "update",
      version: "1.3.0",
      registry: inventory,
      runGenerator: generatedRunner,
    });
    assert.equal(
      readFileSync(join(scope.directory, "notes.md"), "utf8"),
      "# v1.3.0\n\n1.3.0へ更新します。\n",
    );

    // 記録の数が変わってもregistryの更新は要りません。
    writeFileSync(join(scope.directory, "notes.md"), "# v1.3.0\n\n1.3.0と1.3.0。\n");
    assert.doesNotThrow(() =>
      syncReleaseVersion({
        root: scope.root,
        mode: "check",
        version: "1.3.0",
        registry: inventory,
        runGenerator: generatedRunner,
      }),
    );

    // 記録が1件も無い場合は、置換漏れとして拒否します。
    writeFileSync(join(scope.directory, "notes.md"), "# 版の記載なし\n");
    assert.throws(
      () =>
        syncReleaseVersion({
          root: scope.root,
          mode: "check",
          version: "1.3.0",
          registry: inventory,
          runGenerator: generatedRunner,
        }),
      /version記録がありません/,
    );
  } finally {
    scope.cleanup();
  }
});

test("更新と検査を一つのallowlistから決定的に実行する", () => {
  const scope = fixture();
  try {
    const oldFixture = readFileSync(
      join(scope.directory, "fixtures/old-version.json"),
      "utf8",
    );
    const result = syncReleaseVersion({
      root: scope.root,
      mode: "update",
      version: "1.3.0",
      registry: registry(),
      runGenerator: generatedRunner,
    });
    assert.equal(result.current, "1.2.3");
    assert.equal(result.version, "1.3.0");
    assert.deepEqual(
      result.changed.sort(),
      [
        "Cargo.lock",
        "component.txt",
        "generated/protocol.txt",
        "generated/public.txt",
        "release-manifest.json",
      ],
    );
    for (const path of [
      "Cargo.lock",
      "component.txt",
      "generated/protocol.txt",
      "generated/public.txt",
      "release-manifest.json",
    ]) {
      assert.match(readFileSync(join(scope.directory, path), "utf8"), /1\.3\.0/);
      assert.doesNotMatch(
        readFileSync(join(scope.directory, path), "utf8"),
        /1\.2\.3/,
      );
    }
    assert.equal(
      readFileSync(join(scope.directory, "fixtures/old-version.json"), "utf8"),
      oldFixture,
    );
    assert.doesNotThrow(() =>
      syncReleaseVersion({
        root: scope.root,
        mode: "check",
        registry: registry(),
        runGenerator: generatedRunner,
      }),
    );
    assert.deepEqual(
      syncReleaseVersion({
        root: scope.root,
        mode: "update",
        version: "1.3.0",
        registry: registry(),
        runGenerator: generatedRunner,
      }).changed,
      [],
    );
  } finally {
    scope.cleanup();
  }
});

test("管理対象の部分更新を変更前に拒否する", () => {
  const scope = fixture();
  try {
    writeFileSync(join(scope.directory, "component.txt"), "component-version=1.3.0\n");
    const before = readFileSync(join(scope.directory, "release-manifest.json"), "utf8");
    assert.throws(
      () =>
        syncReleaseVersion({
          root: scope.root,
          mode: "update",
          version: "1.3.0",
          registry: registry(),
          runGenerator: generatedRunner,
        }),
      /version記録数/,
    );
    assert.equal(
      readFileSync(join(scope.directory, "release-manifest.json"), "utf8"),
      before,
    );
  } finally {
    scope.cleanup();
  }
});

test("generator失敗時は管理対象と旧version fixtureを復元する", () => {
  const scope = fixture();
  try {
    const before = new Map(
      [
        "release-manifest.json",
        "Cargo.lock",
        "component.txt",
        "generated/protocol.txt",
        "generated/public.txt",
        "fixtures/old-version.json",
      ].map((path) => [
        path,
        readFileSync(join(scope.directory, path), "utf8"),
      ]),
    );
    assert.throws(
      () =>
        syncReleaseVersion({
          root: scope.root,
          mode: "update",
          version: "1.3.0",
          registry: registry(),
          runGenerator: (input) => {
            if (input.id === "public-conformance") {
              throw new Error("generator failure");
            }
            generatedRunner(input);
          },
        }),
      /generator failure/,
    );
    for (const [path, source] of before) {
      assert.equal(readFileSync(join(scope.directory, path), "utf8"), source);
    }
  } finally {
    scope.cleanup();
  }
});

test("第三者packageの版が候補versionと一致しても書き換えない", () => {
  const scope = fixture();
  try {
    // wit-bindgen系のように、外部packageの版が候補versionと偶然一致する
    // lockfileを再現する。cargo-lock対象では登録したlocal packageのblockだけを
    // 書き換えるため、source付きblockは残ります。
    const lockPath = join(scope.directory, "Cargo.lock");
    writeFileSync(
      lockPath,
      readFileSync(lockPath, "utf8") +
        '\n[[package]]\nname = "wit-example"\nversion = "1.3.0"\nsource = "registry+https://example.invalid/index"\n',
    );
    const foreignBlock = /name = "wit-example"\nversion = "1\.3\.0"/;
    assert.doesNotThrow(() =>
      syncReleaseVersion({
        root: scope.root,
        mode: "update",
        version: "1.3.0",
        registry: registry(),
        runGenerator: generatedRunner,
      }),
    );
    assert.match(readFileSync(lockPath, "utf8"), foreignBlock);
  } finally {
    scope.cleanup();
  }
});

test("--checkの副作用を検出して復元する", () => {
  const scope = fixture();
  try {
    const path = join(scope.directory, "generated/protocol.txt");
    const before = readFileSync(path, "utf8");
    assert.throws(
      () =>
        syncReleaseVersion({
          root: scope.root,
          mode: "check",
          registry: registry(),
          runGenerator: ({ id }) => {
            if (id === "protocol") writeFileSync(path, "modified\n");
          },
        }),
      /--checkがfileを変更/,
    );
    assert.equal(readFileSync(path, "utf8"), before);
  } finally {
    scope.cleanup();
  }
});

test("Git worktreeでもgeneratorの未追跡fileを検出して削除する", () => {
  const scope = fixture();
  try {
    assert.equal(spawnSync("git", ["init", "-q"], { cwd: scope.directory }).status, 0);
    assert.equal(spawnSync("git", ["add", "."], { cwd: scope.directory }).status, 0);
    const sideEffect = join(scope.directory, "generated/unexpected.txt");
    assert.throws(
      () =>
        syncReleaseVersion({
          root: scope.root,
          mode: "check",
          registry: registry(),
          runGenerator: ({ id }) => {
            if (id === "protocol") writeFileSync(sideEffect, "unexpected\n");
          },
        }),
      /--checkがfileを変更/,
    );
    assert.equal(existsSync(sideEffect), false);
  } finally {
    scope.cleanup();
  }
});

test("repository内の作業用worktree directoryを副作用の検出から除外する", () => {
  const scope = fixture();
  try {
    assert.equal(spawnSync("git", ["init", "-q"], { cwd: scope.directory }).status, 0);
    assert.equal(spawnSync("git", ["add", "."], { cwd: scope.directory }).status, 0);
    const worktree = join(scope.directory, ".agents", "worker");
    mkdirSync(worktree, { recursive: true });
    assert.equal(spawnSync("git", ["init", "-q"], { cwd: worktree }).status, 0);
    writeFileSync(join(worktree, "release-manifest.json"), '{"packageVersion":"1.2.3"}\n');
    assert.doesNotThrow(() =>
      syncReleaseVersion({
        root: scope.root,
        mode: "check",
        registry: registry(),
        runGenerator: generatedRunner,
      })
    );
  } finally {
    scope.cleanup();
  }
});

test("Gitで追跡中の削除済みfileを検査対象から除外する", () => {
  const scope = fixture();
  try {
    const deleted = join(scope.directory, "obsolete.txt");
    writeFileSync(deleted, "obsolete\n");
    assert.equal(spawnSync("git", ["init", "-q"], { cwd: scope.directory }).status, 0);
    assert.equal(spawnSync("git", ["add", "."], { cwd: scope.directory }).status, 0);
    unlinkSync(deleted);

    assert.doesNotThrow(() =>
      syncReleaseVersion({
        root: scope.root,
        mode: "check",
        registry: registry(),
        runGenerator: generatedRunner,
      })
    );
  } finally {
    scope.cleanup();
  }
});

test("registryの不足、余分、重複、未知fieldと未知generatorを拒否する", () => {
  const cases = [
    (value) => {
      delete value.authority.count;
    },
    (value) => {
      value.targets[0].unknown = true;
    },
    (value) => {
      value.targets.push(structuredClone(value.targets[0]));
    },
    (value) => {
      value.generators[0].id = "unknown";
    },
    (value) => {
      value.generators[0].outputs[0].path = "../outside";
    },
  ];
  for (const mutate of cases) {
    const value = registry();
    mutate(value);
    assert.throws(() => validateRegistry(value));
  }
});

test("stable SemVer以外と不足fileを拒否する", () => {
  for (const version of ["1.2", "v1.2.3", "1.2.3-rc.1", "01.2.3"]) {
    const scope = fixture();
    try {
      assert.throws(
        () =>
          syncReleaseVersion({
            root: scope.root,
            mode: "update",
            version,
            registry: registry(),
            runGenerator: generatedRunner,
          }),
        /stable SemVer/,
      );
    } finally {
      scope.cleanup();
    }
  }

  for (const version of ["1.2.2", "0.99.99"]) {
    const scope = fixture();
    try {
      assert.throws(
        () =>
          syncReleaseVersion({
            root: scope.root,
            mode: "update",
            version,
            registry: registry(),
            runGenerator: generatedRunner,
          }),
        /現在のversionより大きい/,
      );
    } finally {
      scope.cleanup();
    }
  }

  const scope = fixture();
  try {
    rmSync(join(scope.directory, "component.txt"));
    assert.throws(
      () =>
        syncReleaseVersion({
          root: scope.root,
          mode: "check",
          registry: registry(),
          runGenerator: generatedRunner,
        }),
      /管理対象fileがありません/,
    );
  } finally {
    scope.cleanup();
  }
});
