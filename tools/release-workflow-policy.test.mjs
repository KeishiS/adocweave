import assert from "node:assert/strict";
import { test } from "node:test";

import {
  validateNoDirectSecretAccess,
  validatePinnedActions,
  validateProductReleaseRouting,
  validateStandardSourceAndCandidateGates,
  validateWritePermissionGrants,
} from "./release-workflow-policy.mjs";

const CACHE_STEP = { uses: "cachix/cachix-action@sha", with: { authToken: "${{ secrets.CACHIX_AUTH_TOKEN }}" } };
const CACHE_SOURCE = "authToken: ${{ secrets.CACHIX_AUTH_TOKEN }}\n";

function policyInput(name, document, source) {
  return { sources: { [name]: source }, workflows: { [name]: document } };
}

test("外部actionはcommit SHAへ固定する", () => {
  validatePinnedActions({
    "release.yml": {
      jobs: { source: { steps: [{ uses: "actions/checkout@0000000000000000000000000000000000000000" }] } },
    },
  });
  assert.throws(
    () => validatePinnedActions({
      "release.yml": { jobs: { source: { steps: [{ uses: "actions/checkout@v7" }] } } },
    }),
    /not pinned to a full commit SHA/,
  );
});

test("publication以外は明示したread権限だけを持つ", () => {
  validateWritePermissionGrants({
    "release.yml": { permissions: { actions: "read", contents: "read" }, jobs: { source: {} } },
  });
  assert.throws(
    () => validateWritePermissionGrants({
      "release.yml": { permissions: { contents: "read" }, jobs: { source: { permissions: { contents: "write" } } } },
    }),
    /write permissions are reserved for publication/,
  );
  assert.throws(
    () => validateWritePermissionGrants({ "release.yml": { jobs: { source: {} } } }),
    /must declare explicit top-level permissions/,
  );
});

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

function sourceAndCandidateWorkflow() {
  return {
    on: { pull_request: {}, push: { branches: ["main"] } },
    jobs: {
      source: {
        name: "verify",
        steps: [{ run: "nix develop .#ci -c cargo make source-gate" }],
      },
      "main-gate": {
        if: "github.event_name == 'push' && github.ref == 'refs/heads/main'",
        needs: ["source"],
        steps: [{ run: "nix develop .#ci -c cargo make main-gate" }],
      },
      "candidate-plan": {
        needs: ["source", "main-gate"],
        steps: [{ run: "node tools/product-candidate-plan.mjs $GITHUB_OUTPUT" }],
      },
      "build-native": {
        needs: ["candidate-plan"],
        steps: [{ run: "dist build" }],
      },
      "native-smoke": {
        needs: ["build-native"],
        steps: [{ run: "node tools/native-release-smoke.mjs" }],
      },
      "build-global": {
        needs: ["candidate-plan"],
        steps: [{ run: "cargo make global-candidate" }],
      },
      "verify-native-candidate": {
        needs: ["native-smoke"],
        steps: [{ run: "node tools/product-release.mjs --verify-candidate" }],
      },
      "verify-global-candidate": {
        needs: ["build-global"],
        steps: [{ run: "node tools/product-release.mjs --verify-candidate" }],
      },
      "installation-e2e": {
        needs: ["verify-native-candidate"],
        steps: [{ run: "node tools/release-installation-e2e.mjs" }],
      },
      "global-installation-e2e": {
        needs: ["verify-global-candidate"],
        steps: [{ run: "node tools/release-installation-e2e.mjs" }],
      },
    },
  };
}

function validateSourceAndCandidateWorkflow(release = sourceAndCandidateWorkflow()) {
  validateStandardSourceAndCandidateGates(
    { "release.yml": release },
    { "release.yml": JSON.stringify(release) },
  );
}

test("Pull Requestはpath条件なしで標準source gateを1回だけ直接実行する", () => {
  validateSourceAndCandidateWorkflow();

  for (const mutate of [
    (workflow) => { workflow.jobs.source.name = "source"; },
    (workflow) => { workflow.jobs.source.if = "github.event_name == 'pull_request'"; },
    (workflow) => { workflow.jobs.source.uses = "./.github/workflows/quality.yml"; },
    (workflow) => { workflow.jobs["main-gate"].steps.push({ run: "cargo make source-gate" }); },
  ]) {
    const workflow = sourceAndCandidateWorkflow();
    mutate(workflow);
    assert.throws(() => validateSourceAndCandidateWorkflow(workflow), /source|必須check/);
  }
});

test("削除したpath分類と手動aggregateをworkflowへ戻さない", () => {
  const mutations = [
    ["native-change-plan", (workflow) => { workflow.jobs.source.steps.push({ run: "node tools/native-change-plan.mjs" }); }],
    ["git diff", (workflow) => { workflow.jobs.source.steps.push({ run: "git diff --name-only HEAD^" }); }],
    ["candidate_required", (workflow) => { workflow.jobs.source.env = { candidate_required: "true" }; }],
    ["quality input", (workflow) => { workflow.jobs.source.with = { run_rust_source: true }; }],
    ["not reachable", (workflow) => { workflow.jobs.source.steps.push({ run: "echo not reachable" }); }],
    ["always aggregate", (workflow) => { workflow.jobs.source.if = "always()"; }],
    ["PR candidate", (workflow) => { workflow.jobs["candidate-plan"].strategy = { matrix: { product: ["pr"], artifact_key: ["local"] } }; }],
  ];
  for (const [name, mutate] of mutations) {
    const workflow = sourceAndCandidateWorkflow();
    mutate(workflow);
    assert.throws(
      () => validateSourceAndCandidateWorkflow(workflow),
      undefined,
      `${name}を拒否しませんでした`,
    );
  }

  const workflow = sourceAndCandidateWorkflow();
  workflow.jobs["merge-gate"] = { if: "success()", steps: [{ run: "true" }] };
  assert.throws(() => validateSourceAndCandidateWorkflow(workflow), /削除済みjob merge-gate/);
});

test("main candidate planはsourceとmain gateの成功後に製品計画を1回だけ生成する", () => {
  for (const mutate of [
    (workflow) => { workflow.jobs["candidate-plan"].needs = ["main-gate"]; },
    (workflow) => { workflow.jobs["candidate-plan"].steps = [{ run: "node custom-plan.mjs" }]; },
    (workflow) => { workflow.jobs["candidate-plan"].steps.push({ run: "node tools/product-candidate-plan.mjs" }); },
    (workflow) => { delete workflow.jobs["main-gate"].if; },
  ]) {
    const workflow = sourceAndCandidateWorkflow();
    mutate(workflow);
    assert.throws(() => validateSourceAndCandidateWorkflow(workflow), /candidate-plan|main-gate/);
  }
});

test("成果物のbuild、smoke、installationおよびcandidate処理をmainへ限定する", () => {
  const workflow = sourceAndCandidateWorkflow();
  workflow.jobs["build-native"].needs = ["source"];
  assert.throws(
    () => validateSourceAndCandidateWorkflow(workflow),
    /成果物job build-nativeはmain candidate経路/,
  );
});
