import assert from "node:assert/strict";
import test from "node:test";

import { installLatestCompatibleConsumer } from "./textlint-plugin-consumer-e2e.mjs";
import { runTextlintPluginCompatibilityProbe } from "./textlint-plugin-compatibility-probe.mjs";

test("公開manifestのidentityと対応版でlatest依存を観測する", async () => {
  const manifest = {
    name: "@adocweave/textlint-plugin-asciidoc",
    peerDependencies: { textlint: "15.8.0" },
  };
  const calls = [];
  const result = await runTextlintPluginCompatibilityProbe("candidate.tgz", {
    manifest,
    runConsumer: async (archive, options) => calls.push({ archive, options }),
  });
  assert.equal(calls.length, 1);
  assert.match(calls[0].archive, /candidate\.tgz$/);
  assert.equal(calls[0].options.manifest, manifest);
  assert.equal(calls[0].options.installPackage, installLatestCompatibleConsumer);
  assert.deepEqual(result, {
    packageName: "@adocweave/textlint-plugin-asciidoc",
    textlintVersion: "15.8.0",
  });
});
