import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import test from "node:test";

import {
  expectedReleaseAssets,
  validateDistPlan,
  validateReleaseTag,
  verifyRepository,
} from "./release-contract.mjs";

const release = verifyRepository();
const plan = JSON.parse(execFileSync("dist", ["plan", `--tag=${release.tag}`, "--output-format=json"], {
  encoding: "utf8",
}));

test("repositoryは一つのworkspace版とtagを使う", () => {
  assert.deepEqual(release, { tag: "v0.56.0", version: "0.56.0" });
  assert.deepEqual(validateReleaseTag(release.tag), release);
  for (const tag of ["0.53.0", "adocweave-cli/v0.53.0", "adocweave-lsp/v0.53.0", "v0.53.0-rc.1"]) {
    assert.throws(() => validateReleaseTag(tag), /exactly/);
  }
});

test("cargo-dist planは単一appとnative releaseの成果物を持つ", () => {
  const validated = validateDistPlan(plan, release.tag);
  assert.equal(validated.version, release.version);
  assert.deepEqual(validated.assets, expectedReleaseAssets());
  assert.equal(plan.releases.length, 1);
  assert.match(plan.announcement_github_body, /### Rust API/);
});

test("native archiveはadocweaveだけを含む", () => {
  const names = Object.values(plan.artifacts)
    .filter(({ kind }) => kind === "executable-zip")
    .map(({ assets }) => assets.filter(({ kind }) => kind === "executable").map(({ name }) => name));
  assert.deepEqual(names, [["adocweave"], ["adocweave"], ["adocweave"], ["adocweave"]]);
});

test("製品別または不完全なplanを拒否する", () => {
  assert.throws(
    () => validateDistPlan({ ...plan, releases: [{ ...plan.releases[0], app_name: "adocweave-cli" }] }),
    /unified adocweave app/,
  );
  assert.throws(
    () => validateDistPlan({ ...plan, github_attestations: false }),
    /attest every hosted artifact/,
  );
});
