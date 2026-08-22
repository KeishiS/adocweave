import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import {
  validateNoDirectSecretAccess,
  validateGateTaskContract,
  validatePinnedActions,
  validateProductReleaseRouting,
  validateStandardSourceAndCandidateGates,
  validateWritePermissionGrants,
} from "./release-workflow-policy.mjs";

const makefile = readFileSync(new URL("../Makefile.toml", import.meta.url), "utf8");

function mutateTask(source, name, mutate) {
  const heading = `[tasks.${name}]`;
  const start = source.indexOf(heading);
  const next = source.indexOf("\n[tasks.", start + heading.length);
  const end = next < 0 ? source.length : next;
  return `${source.slice(0, start)}${mutate(source.slice(start, end))}${source.slice(end)}`;
}

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
        steps: [{ run: "nix develop .#ci-fuzz -c cargo make main-gate" }],
      },
      "candidate-plan": {
        if: "github.event_name == 'push' && github.ref == 'refs/heads/main'",
        needs: ["source", "main-gate"],
        steps: [{ run: 'node tools/product-candidate-plan.mjs "$GITHUB_OUTPUT"' }],
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
        steps: [{ run: "nix develop .#ci-browser -c cargo make test-global-product-candidate" }],
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
    (workflow) => { workflow.on.pull_request.paths = ["docs/**"]; },
    (workflow) => { workflow.on.push.paths = ["src/**"]; },
    (workflow) => { workflow.jobs.source.name = "source"; },
    (workflow) => { workflow.jobs.source.if = "github.event_name == 'pull_request'"; },
    (workflow) => { workflow.jobs.source.needs = ["main-gate"]; },
    (workflow) => { workflow.jobs.source.strategy = { matrix: { shard: [1, 2] } }; },
    (workflow) => { workflow.jobs.source.uses = "./.github/workflows/quality.yml"; },
    (workflow) => { workflow.jobs.source.steps[0].run += " || true"; },
    (workflow) => { workflow.jobs["main-gate"].steps.push({ run: "cargo make source-gate" }); },
  ]) {
    const workflow = sourceAndCandidateWorkflow();
    mutate(workflow);
    assert.throws(() => validateSourceAndCandidateWorkflow(workflow), /source|必須check/);
  }

  const eventTypes = sourceAndCandidateWorkflow();
  eventTypes.on.pull_request.types = ["opened", "synchronize", "reopened"];
  validateSourceAndCandidateWorkflow(eventTypes);
});

test("削除したpath分類と手動aggregateをworkflowへ戻さない", () => {
  const mutations = [
    ["native-change-plan", (workflow) => { workflow.jobs.source.steps.push({ run: "node tools/native-change-plan.mjs" }); }],
    ["git diff", (workflow) => { workflow.jobs.source.steps.push({ run: "git diff --name-only HEAD^" }); }],
    ["candidate_required", (workflow) => { workflow.jobs.source.env = { candidate_required: "true" }; }],
    ["quality input", (workflow) => { workflow.jobs.source.with = { run_rust_source: true }; }],
    ["not reachable", (workflow) => { workflow.jobs.source.steps.push({ run: "echo not reachable" }); }],
    ["always aggregate", (workflow) => { workflow.jobs.source.if = "always()"; }],
    ["result aggregate", (workflow) => { workflow.jobs["build-native"].if = "needs.main-gate.result == 'success'"; }],
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
    (workflow) => { workflow.jobs["main-gate"].if += " || github.event_name == 'pull_request'"; },
    (workflow) => { workflow.jobs["main-gate"].if = `failure() && ${workflow.jobs["main-gate"].if}`; },
    (workflow) => { workflow.jobs["main-gate"].steps = []; },
    (workflow) => { workflow.jobs["main-gate"].steps[0].run += " || true"; },
    (workflow) => { workflow.jobs.source.steps.push({ run: "cargo make main-gate" }); },
    (workflow) => { workflow.jobs["candidate-plan"].if = "failure()"; },
  ]) {
    const workflow = sourceAndCandidateWorkflow();
    mutate(workflow);
    assert.throws(() => validateSourceAndCandidateWorkflow(workflow), /source|candidate-plan|main-gate/);
  }

  const reordered = sourceAndCandidateWorkflow();
  reordered.jobs["candidate-plan"].needs.reverse();
  validateSourceAndCandidateWorkflow(reordered);
});

test("成果物のbuild、smoke、installationおよびcandidate処理をmainへ限定する", () => {
  for (const mutate of [
    (workflow) => { workflow.jobs["build-native"].needs = ["source"]; },
    (workflow) => { delete workflow.jobs["build-global"]; },
    (workflow) => { workflow.jobs["installation-e2e"].if = "failure()"; },
    (workflow) => { workflow.jobs["preview-candidate"] = { needs: ["source"], steps: [{ run: "true" }] }; },
  ]) {
    const workflow = sourceAndCandidateWorkflow();
    mutate(workflow);
    assert.throws(
      () => validateSourceAndCandidateWorkflow(workflow),
      /成果物job/,
    );
  }
});

test("global成果物は製品別の完成candidate taskで検査する", () => {
  const workflow = sourceAndCandidateWorkflow();
  workflow.jobs["build-global"].steps[0].run += " || true";
  assert.throws(
    () => validateSourceAndCandidateWorkflow(workflow),
    /製品別の完成candidate task/,
  );
});

test("sourceとmainの標準task境界をMakefileで固定する", () => {
  validateGateTaskContract(makefile);
  validateGateTaskContract(
    mutateTask(makefile, "main-gate", (body) => body.replace('  "nix-package-check",\n]', '  "nix-package-check",\n  "test-grammar",\n]')),
  );

  for (const [name, mutate] of [
    ["source dependency", (source) => mutateTask(source, "source-gate", (body) => body.replace('  "fmt-check",\n', ""))],
    ["main dependency", (source) => mutateTask(source, "main-gate", (body) => body.replace('  "fuzz",\n', ""))],
    ["main candidate", (source) => mutateTask(source, "main-gate", (body) => body.replace('  "nix-package-check",\n]', '  "nix-package-check",\n  "release-global-candidate",\n]'))],
    ["verify alias", (source) => mutateTask(source, "verify", (body) => body.replace('alias = "source-gate"', 'alias = "main-gate"'))],
    ["acceptance", (source) => mutateTask(source, "acceptance", (body) => body.replace('  "source-gate",\n', ""))],
    ["release-check", (source) => mutateTask(source, "release-check", (body) => body.replace('  "main-gate",\n', ""))],
  ]) {
    assert.throws(
      () => validateGateTaskContract(mutate(makefile)),
      undefined,
      `${name}の退行を拒否しませんでした`,
    );
  }
});
