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

const script = new URL("normalize-darwin-archives.sh", import.meta.url);
const target = "aarch64-apple-darwin";
const archiveName = `adocweave-${target}.zip`;

function command(commandName, args, options = {}) {
  const result = spawnSync(commandName, args, { encoding: "utf8", ...options });
  assert.equal(result.error, undefined, result.error?.message);
  return result;
}

function plan(targetTriple = target, overrides = {}) {
  const artifact = {
    name: `adocweave-${targetTriple}.zip`,
    kind: "executable-zip",
    target_triples: [targetTriple],
    assets: [
      { name: "adocweave", path: "adocweave", kind: "executable" },
      { name: "README.adoc", path: "README.adoc", kind: "readme" },
    ],
    ...overrides,
  };
  return {
    releases: [{ app_name: "adocweave", app_version: "0.51.0" }],
    artifacts: { [artifact.name]: artifact },
  };
}

function fixture(planValue = plan()) {
  const root = mkdtempSync(join(tmpdir(), "adocweave-darwin-normalize-test."));
  const artifacts = join(root, "artifacts");
  const mockBin = join(root, "bin");
  const otoolLog = join(root, "otool.log");
  const planFile = join(root, "dist-plan.json");
  mkdirSync(artifacts);
  mkdirSync(mockBin);
  writeFileSync(planFile, JSON.stringify(planValue));
  const otool = join(mockBin, "otool");
  writeFileSync(
    otool,
    '#!/usr/bin/env bash\nprintf "%s\\n" "${@: -1}" >> "$MOCK_OTOOL_LOG"\nprintf "%s:\\n" "${@: -1}"\n',
  );
  chmodSync(otool, 0o755);
  return { root, artifacts, mockBin, otoolLog, planFile };
}

function addArchive(root, artifacts) {
  const stage = join(root, "stage");
  mkdirSync(stage);
  writeFileSync(join(stage, "adocweave"), "binary\n");
  writeFileSync(join(stage, "README.adoc"), "readme\n");
  const result = command("zip", ["-q", "-X", join(artifacts, archiveName), "adocweave", "README.adoc"], {
    cwd: stage,
  });
  assert.equal(result.status, 0, result.stderr);
}

function normalize(fixtureValue, requestedTarget = target) {
  const { artifacts, mockBin, otoolLog, planFile } = fixtureValue;
  return command("bash", [script.pathname, artifacts, planFile, requestedTarget], {
    env: { ...process.env, MOCK_OTOOL_LOG: otoolLog, PATH: `${mockBin}:${process.env.PATH}` },
  });
}

test("cargo-distの計画にある単一Darwin archiveを正規化する", () => {
  const value = fixture();
  try {
    addArchive(value.root, value.artifacts);
    const result = normalize(value);
    assert.equal(result.status, 0, result.stderr);
    const entries = command("unzip", ["-Z1", join(value.artifacts, archiveName)]);
    assert.deepEqual(entries.stdout.trim().split("\n").sort(), ["README.adoc", "adocweave"]);
    const inspected = readFileSync(value.otoolLog, "utf8").trim().split("\n");
    assert.equal(inspected.length, 2);
    assert.equal(inspected.every((path) => path.endsWith("/archive/adocweave")), true);
  } finally {
    rmSync(value.root, { recursive: true, force: true });
  }
});

test("計画にあるDarwin archiveの欠落を拒否する", () => {
  const value = fixture();
  try {
    const result = normalize(value);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /adocweave-aarch64-apple-darwin\.zip/);
  } finally {
    rmSync(value.root, { recursive: true, force: true });
  }
});

test("Darwin以外のtargetを拒否する", () => {
  const value = fixture();
  try {
    const result = normalize(value, "x86_64-unknown-linux-musl");
    assert.equal(result.status, 2);
    assert.match(result.stderr, /Darwin以外のtarget/);
  } finally {
    rmSync(value.root, { recursive: true, force: true });
  }
});

test("計画にないDarwin targetを拒否する", () => {
  const value = fixture();
  try {
    const result = normalize(value, "x86_64-apple-darwin");
    assert.equal(result.status, 2);
    assert.match(result.stderr, /一つに特定できません/);
  } finally {
    rmSync(value.root, { recursive: true, force: true });
  }
});

test("単一adocweave実行ファイルではない計画を拒否する", () => {
  const value = fixture(plan(target, {
    assets: [
      { name: "adocweave", path: "adocweave", kind: "executable" },
      { name: "other", path: "other", kind: "executable" },
    ],
  }));
  try {
    const result = normalize(value);
    assert.equal(result.status, 2);
    assert.match(result.stderr, /一つに特定できません/);
  } finally {
    rmSync(value.root, { recursive: true, force: true });
  }
});
