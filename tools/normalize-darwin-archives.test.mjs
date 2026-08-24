import assert from "node:assert/strict";
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { test } from "node:test";

const repository = new URL("..", import.meta.url);
const script = new URL("normalize-darwin-archives.sh", import.meta.url);
const target = "aarch64-apple-darwin";
const products = {
  cli: { archive: `adocweave-cli-${target}.zip`, executable: "adocweave" },
  lsp: { archive: `adocweave-lsp-${target}.zip`, executable: "adocweave-lsp" },
};

function command(commandName, args, options = {}) {
  const result = spawnSync(commandName, args, { encoding: "utf8", ...options });
  assert.equal(result.error, undefined, result.error?.message);
  return result;
}

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "adocweave-darwin-normalize-test."));
  const artifacts = join(root, "artifacts");
  const mockBin = join(root, "bin");
  const otoolLog = join(root, "otool.log");
  mkdirSync(artifacts);
  mkdirSync(mockBin);
  const otool = join(mockBin, "otool");
  writeFileSync(
    otool,
    '#!/usr/bin/env bash\nprintf "%s\\n" "${@: -1}" >> "$MOCK_OTOOL_LOG"\nprintf "%s:\\n" "${@: -1}"\n',
  );
  chmodSync(otool, 0o755);
  return { root, artifacts, mockBin, otoolLog };
}

function addArchive(root, artifacts, product) {
  const { archive, executable } = products[product];
  const stage = join(root, `stage-${product}`);
  mkdirSync(stage);
  writeFileSync(join(stage, executable), `${product}\n`);
  writeFileSync(join(stage, "LICENSE-APACHE"), "license\n");
  writeFileSync(join(stage, "README.adoc"), "readme\n");
  writeFileSync(join(stage, "THIRD_PARTY_NOTICES.adoc"), "notices\n");
  const result = command("zip", ["-q", "-X", join(artifacts, archive),
    executable, "LICENSE-APACHE", "README.adoc", "THIRD_PARTY_NOTICES.adoc"], { cwd: stage });
  assert.equal(result.status, 0, result.stderr);
}

function normalize(artifacts, mockBin, otoolLog, product) {
  return command("bash", [script.pathname, artifacts, product, target], {
    cwd: repository,
    env: { ...process.env, MOCK_OTOOL_LOG: otoolLog, PATH: `${mockBin}:${process.env.PATH}` },
  });
}

for (const product of Object.keys(products)) {
  test(`${product}だけのDarwin archiveを正規化する`, () => {
    const { root, artifacts, mockBin, otoolLog } = fixture();
    try {
      addArchive(root, artifacts, product);
      const result = normalize(artifacts, mockBin, otoolLog, product);
      assert.equal(result.status, 0, result.stderr);
      assert.equal(readFileSync(join(artifacts, products[product].archive)).length > 0, true);
      const entries = command("unzip", ["-Z1", join(artifacts, products[product].archive)]);
      assert.deepEqual(entries.stdout.trim().split("\n").sort(), [
        "LICENSE-APACHE",
        "README.adoc",
        "THIRD_PARTY_NOTICES.adoc",
        products[product].executable,
      ].sort());
      const inspected = readFileSync(otoolLog, "utf8").trim().split("\n");
      assert.equal(inspected.length, 2);
      assert.equal(
        inspected.every((path) => path.endsWith(
          `/${products[product].executable}/${products[product].executable}`,
        )),
        true,
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
}

test("選択した製品のarchive欠落を拒否する", () => {
  const { root, artifacts, mockBin, otoolLog } = fixture();
  try {
    const result = normalize(artifacts, mockBin, otoolLog, "lsp");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /adocweave-lsp-aarch64-apple-darwin\.zip/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("Darwin実行fileを持たない製品を拒否する", () => {
  const { root, artifacts, mockBin, otoolLog } = fixture();
  try {
    const result = normalize(artifacts, mockBin, otoolLog, "wasm");
    assert.equal(result.status, 2);
    assert.match(result.stderr, /Darwin実行fileを持つ製品ではありません/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("未知の製品を拒否する", () => {
  const { root, artifacts, mockBin, otoolLog } = fixture();
  try {
    const result = normalize(artifacts, mockBin, otoolLog, "unknown");
    assert.equal(result.status, 2);
    assert.match(result.stderr, /Darwin実行fileを持つ製品ではありません/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("別製品のDarwin archiveが混在する場合は拒否する", () => {
  const { root, artifacts, mockBin, otoolLog } = fixture();
  try {
    addArchive(root, artifacts, "cli");
    addArchive(root, artifacts, "lsp");
    const result = normalize(artifacts, mockBin, otoolLog, "lsp");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /別製品のDarwin archiveが混在しています/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("配布計画にないDarwin風targetを拒否する", () => {
  const { root, artifacts, mockBin, otoolLog } = fixture();
  try {
    const result = command("bash", [script.pathname, artifacts, "lsp", "x86_64-apple-darwin"], {
      cwd: repository,
      env: { ...process.env, MOCK_OTOOL_LOG: otoolLog, PATH: `${mockBin}:${process.env.PATH}` },
    });
    assert.equal(result.status, 2);
    assert.match(result.stderr, /配布計画にDarwin targetがありません/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
