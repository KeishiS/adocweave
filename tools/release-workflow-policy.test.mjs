import assert from "node:assert/strict";
import { test } from "node:test";

import {
  validateNoDirectSecretAccess,
  validateProductReleaseRouting,
  validateTextlintReleaseGates,
} from "./release-workflow-policy.mjs";

const CACHE_STEP = { uses: "cachix/cachix-action@sha", with: { authToken: "${{ secrets.CACHIX_AUTH_TOKEN }}" } };
const CACHE_SOURCE = "authToken: ${{ secrets.CACHIX_AUTH_TOKEN }}\n";

function policyInput(name, document, source) {
  return { sources: { [name]: source }, workflows: { [name]: document } };
}

test("the binary cache job may read the Cachix write token", () => {
  const { sources, workflows } = policyInput(
    "release-dispatch.yml",
    { jobs: { "binary-cache": { steps: [CACHE_STEP] } } },
    CACHE_SOURCE,
  );
  validateNoDirectSecretAccess(sources, workflows);
});

const OPEN_VSX_STEP = { env: { OPEN_VSX_TOKEN: "${{ secrets.OPEN_VSX_TOKEN }}" }, run: "curl ..." };
const OPEN_VSX_SOURCE = "OPEN_VSX_TOKEN: ${{ secrets.OPEN_VSX_TOKEN }}\n";

test("the Open VSX job may read the registry publish token", () => {
  const { sources, workflows } = policyInput(
    "open-vsx-publish.yml",
    { jobs: { publish: { steps: [OPEN_VSX_STEP] } } },
    OPEN_VSX_SOURCE,
  );
  validateNoDirectSecretAccess(sources, workflows);
});

test("the Open VSX token outside its workflow is rejected", () => {
  const { sources, workflows } = policyInput(
    "release-dispatch.yml",
    { jobs: { "binary-cache": { steps: [OPEN_VSX_STEP] } } },
    OPEN_VSX_SOURCE,
  );
  assert.throws(
    () => validateNoDirectSecretAccess(sources, workflows),
    /job binary-cache reads secrets\.OPEN_VSX_TOKEN/,
  );
});

test("workflows without secret references pass", () => {
  const { sources, workflows } = policyInput(
    "release.yml",
    { jobs: { build: { steps: [{ run: "echo ${{ github.token }}" }] } } },
    "run: echo ${{ github.token }}\n",
  );
  validateNoDirectSecretAccess(sources, workflows);
});

test("another secret in the binary cache job is rejected", () => {
  const { sources, workflows } = policyInput(
    "release-dispatch.yml",
    { jobs: { "binary-cache": { steps: [{ run: "echo ${{ secrets.OTHER_TOKEN }}" }] } } },
    "run: echo ${{ secrets.OTHER_TOKEN }}\n",
  );
  assert.throws(() => validateNoDirectSecretAccess(sources, workflows), /reads secrets\.OTHER_TOKEN/);
});

test("the Cachix token outside the binary cache job is rejected", () => {
  const { sources, workflows } = policyInput(
    "release-dispatch.yml",
    { jobs: { publish: { steps: [CACHE_STEP] } } },
    CACHE_SOURCE,
  );
  assert.throws(
    () => validateNoDirectSecretAccess(sources, workflows),
    /job publish reads secrets\.CACHIX_AUTH_TOKEN/,
  );
});

test("the Cachix token in another workflow is rejected", () => {
  const { sources, workflows } = policyInput(
    "release.yml",
    { jobs: { "binary-cache": { steps: [CACHE_STEP] } } },
    CACHE_SOURCE,
  );
  assert.throws(
    () => validateNoDirectSecretAccess(sources, workflows),
    /release\.yml job binary-cache reads secrets\.CACHIX_AUTH_TOKEN/,
  );
});

test("a secret reference outside every job is rejected", () => {
  const { sources, workflows } = policyInput(
    "release-dispatch.yml",
    {
      env: { CACHIX_AUTH_TOKEN: "${{ secrets.CACHIX_AUTH_TOKEN }}" },
      jobs: { "binary-cache": { steps: [{ run: "cachix push keishis result" }] } },
    },
    "env:\n  CACHIX_AUTH_TOKEN: ${{ secrets.CACHIX_AUTH_TOKEN }}\n",
  );
  assert.throws(() => validateNoDirectSecretAccess(sources, workflows), /outside an allowed job/);
});

function productRoutingWorkflows() {
  return {
    "release-dispatch.yml": {
      on: {
        workflow_dispatch: {
          inputs: {
            product: {
              options: ["cli", "lsp", "browser", "textlint", "vscode", "zed"],
              required: true,
              type: "choice",
            },
          },
        },
      },
      jobs: {
        readiness: { outputs: { product: "${{ steps.readiness.outputs.product }}" } },
        plan: {
          steps: [{
            id: "plan",
            run: 'if [ "$build" = cargo-dist ]; then\n  node product-release --publication-plan "$PRODUCT"\nfi',
          }],
        },
        publish: { with: { product: "${{ needs.readiness.outputs.product }}" } },
        "textlint-plugin-post-release-smoke": {
          if: "needs.readiness.outputs.product == 'textlint'",
        },
        "open-vsx": { if: "needs.readiness.outputs.product == 'vscode'" },
        "binary-cache": { if: "needs.readiness.outputs.product == 'cli'" },
      },
    },
    "release-publish.yml": {
      on: { workflow_call: { inputs: { product: { required: true, type: "string" } } } },
      jobs: {
        publish: {
          steps: [
            {
              uses: "actions/download-artifact@0000000000000000000000000000000000000000",
              with: { name: "release-candidate-${{ inputs.product }}" },
            },
            {
              name: "Immutable release input verification",
              run: 'node product-release --verify-publication "$PRODUCT"',
            },
          ],
        },
      },
    },
  };
}

test("product release routing accepts the separated product contracts", () => {
  validateProductReleaseRouting(productRoutingWorkflows());
});

test("product release routing rejects a post-release job shared by every product", () => {
  const workflows = productRoutingWorkflows();
  workflows["release-dispatch.yml"].jobs["open-vsx"].if = "success()";
  assert.throws(() => validateProductReleaseRouting(workflows), /open-vsx must run only for vscode/);
});

test("product release routing rejects a generic candidate artifact", () => {
  const workflows = productRoutingWorkflows();
  workflows["release-publish.yml"].jobs.publish.steps[0].with.name = "release-candidate";
  assert.throws(
    () => validateProductReleaseRouting(workflows),
    /download only the selected product candidate/,
  );
});

test("textlintのPR検査を完成archiveと固定consumerの各1回に限定する", () => {
  const workflows = {
    "release.yml": {
      jobs: {
        "global-installation-e2e": {
          steps: [{
            name: "Global installation and complete removal",
            run: "for product in browser vscode zed; do verify $product; done",
          }],
        },
      },
    },
  };
  const makefile = `
[tasks.release-global-artifacts]
dependencies = ["test-textlint-plugin-release-package"]
[tasks.release-global-candidate]
dependencies = ["release-global-artifacts", "textlint-plugin-release-consumer-e2e"]
[tasks.release-installation-e2e-host]
dependencies = ["native-release-smoke-host", "package-browser-release"]
`;
  validateTextlintReleaseGates(workflows, makefile);

  workflows["release.yml"].jobs["textlint-plugin-installation-e2e"] = {};
  assert.throws(
    () => validateTextlintReleaseGates(workflows, makefile),
    /must not duplicate the fixed textlint consumer E2E/,
  );
});
