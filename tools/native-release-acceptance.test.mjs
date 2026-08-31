import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { validatePublishedNativeRelease } from "./native-release-acceptance.mjs";
import {
  expectedPublishedReleaseAssets,
  expectedReleaseAssets,
  verifyRepository,
} from "./native-release-checks.mjs";
import { nativeReleasePlanFixture } from "./native-release-plan.fixture.mjs";

const repository = verifyRepository();
const plan = JSON.stringify(nativeReleasePlanFixture(repository));

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function fixture() {
  const directory = mkdtempSync(join(tmpdir(), "adocweave-native-release-"));
  const archives = expectedReleaseAssets().filter((name) => name.endsWith(".zip"));
  const sums = [];
  for (const name of archives) {
    const bytes = Buffer.from(`fixture:${name}`);
    const sum = sha256(bytes);
    writeFileSync(join(directory, name), bytes);
    writeFileSync(join(directory, `${name}.sha256`), `${sum}  ${name}\n`);
    sums.push(`${sum}  ${name}`);
  }
  writeFileSync(join(directory, "sha256.sum"), `${sums.join("\n")}\n`);
  writeFileSync(join(directory, "dist-manifest.json"), plan);
  const release = {
    assets: expectedPublishedReleaseAssets().map((name) => ({ name, size: 1 })),
    draft: false,
    prerelease: false,
    tag_name: repository.tag,
  };
  return { directory, release };
}

test("公開済みnative Releaseの成果物、manifest、checksumを検証する", (context) => {
  const candidate = fixture();
  context.after(() => rmSync(candidate.directory, { force: true, recursive: true }));
  assert.deepEqual(
    validatePublishedNativeRelease({ ...candidate, tag: repository.tag }),
    {
      assets: expectedPublishedReleaseAssets(),
      manifest: "dist-manifest.json",
      tag: repository.tag,
    },
  );
});

test("余分な成果物と不正なchecksumを拒否する", (context) => {
  const extra = fixture();
  context.after(() => rmSync(extra.directory, { force: true, recursive: true }));
  extra.release.assets.push({ name: "adocweave-wasm.tgz", size: 1 });
  assert.throws(
    () => validatePublishedNativeRelease({ ...extra, tag: repository.tag }),
    /asset set mismatch/u,
  );

  const changed = fixture();
  context.after(() => rmSync(changed.directory, { force: true, recursive: true }));
  const archive = expectedReleaseAssets().find((name) => name.endsWith(".zip"));
  writeFileSync(join(changed.directory, archive), "changed");
  assert.throws(
    () => validatePublishedNativeRelease({ ...changed, tag: repository.tag }),
    /individual checksum mismatch/u,
  );
});
