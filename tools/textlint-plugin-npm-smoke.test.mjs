import assert from "node:assert/strict";
import test from "node:test";

import {
  assertSignatureAudit,
  runTextlintPluginNpmSmoke,
} from "./textlint-plugin-npm-smoke.mjs";

const manifest = { name: "@adocweave/textlint-plugin-asciidoc", version: "0.47.0" };

function recorder() {
  const calls = [];
  return { calls, run: async (spec, options) => calls.push({ spec, options }) };
}

test("registryのversionを解決してからclean installとnpxを観測する", async () => {
  const consumer = recorder();
  const npx = recorder();
  const audited = [];
  const requested = [];
  const published = await runTextlintPluginNpmSmoke({
    manifest,
    fetchJson: async (url) => {
      requested.push(url);
      return { name: manifest.name, version: manifest.version };
    },
    audit: async ({ name, version }) => audited.push(`${name}@${version}`),
    runConsumerE2E: consumer.run,
    runNpxSmoke: npx.run
  });
  assert.deepEqual(published, manifest);
  assert.deepEqual(requested, [
    "https://registry.npmjs.org/@adocweave/textlint-plugin-asciidoc/0.47.0"
  ]);
  const spec = "@adocweave/textlint-plugin-asciidoc@0.47.0";
  assert.deepEqual(audited, [spec]);
  assert.deepEqual(consumer.calls, [{ spec, options: { manifest } }]);
  assert.deepEqual(npx.calls, [{ spec, options: { manifest } }]);
});

test("公開前に実行した場合は導入を試さず止める", async () => {
  const consumer = recorder();
  await assert.rejects(
    runTextlintPluginNpmSmoke({
      manifest,
      fetchJson: async () => ({ error: "Not found" }),
      audit: async () => {},
      runConsumerE2E: consumer.run,
      runNpxSmoke: consumer.run
    }),
    /is not available from the npm Registry/u
  );
  assert.equal(consumer.calls.length, 0);
});

test("registryが別のversionを返した場合を拒否する", async () => {
  await assert.rejects(
    runTextlintPluginNpmSmoke({
      manifest,
      fetchJson: async () => ({ name: manifest.name, version: "0.46.2" }),
      audit: async () => {},
      runConsumerE2E: async () => {},
      runNpxSmoke: async () => {}
    }),
    /is not available from the npm Registry/u
  );
});

test("対象packageの署名付きSLSA provenanceを要求する", () => {
  const provenance = {
    predicateType: "https://slsa.dev/provenance/v1",
    bundle: { dsseEnvelope: { signatures: [{ sig: "verified" }] } },
  };
  assert.doesNotThrow(() => assertSignatureAudit({
    invalid: [],
    missing: [],
    verified: [{
      name: manifest.name,
      version: manifest.version,
      attestations: { provenance: { predicateType: provenance.predicateType } },
      attestationBundles: [provenance],
    }],
  }, manifest));
  assert.throws(() => assertSignatureAudit({
    invalid: [],
    missing: [],
    verified: [],
  }, manifest), /does not have a verified attestation/u);
  assert.throws(() => assertSignatureAudit({
    invalid: [],
    missing: [],
    verified: [{
      name: manifest.name,
      version: manifest.version,
      attestations: { provenance: { predicateType: provenance.predicateType } },
      attestationBundles: [{ ...provenance, bundle: { dsseEnvelope: { signatures: [] } } }],
    }],
  }, manifest), /does not have signed provenance/u);
});
