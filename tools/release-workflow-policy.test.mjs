import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import {
  validateCargoMakeReferences,
  validateBuildReuseContract,
  validateNoDirectSecretAccess,
  validateGateTaskContract,
  validatePinnedActions,
  validateProductReleaseRouting,
  validateReleaseGuideContract,
  validateStandardSourceAndCandidateGates,
  validateWritePermissionGrants,
} from "./release-workflow-policy.mjs";

const makefile = readFileSync(new URL("../Makefile.toml", import.meta.url), "utf8");

test("利用者向け文書とmessageは開発者向けcargo-make taskだけを案内する", () => {
  validateCargoMakeReferences({ "guide.adoc": "cargo make verify\ncargo make acceptance\ncargo make\n" });
  assert.throws(
    () => validateCargoMakeReferences({ "guide.adoc": "cargo make html5-check\n" }),
    /開発者向けではない.*html5-check/,
  );
  assert.throws(
    () => validateCargoMakeReferences({ "guide.adoc": "cargo make main-gate\n" }),
    /開発者向けではない.*main-gate/,
  );
});

test("Release手順は製品別の同期と雛形を正本にする", () => {
  const guide = `
node tools/sync-release-version.mjs --product PRODUCT --version X.Y.Z
release/notes.template.md
node tools/release-notes.mjs --check PRODUCT
node tools/sync-release-version.mjs --product PRODUCT --check
`;
  assert.doesNotThrow(() => validateReleaseGuideContract(guide));
  assert.throws(
    () => validateReleaseGuideContract(guide.replace("--product PRODUCT --version", "--version")),
    /使用方法/,
  );
  assert.throws(
    () => validateReleaseGuideContract(guide.replace("release/notes.template.md", "release/notes.md")),
    /雛形/,
  );
  assert.throws(
    () => validateReleaseGuideContract(guide.replace("--check PRODUCT", "--check PRODUCT EXTRA")),
    /使用方法/,
  );
  assert.throws(
    () => validateReleaseGuideContract(`${guide}\n## 対応関係\n`),
    /見出しを手順書へ複製/,
  );
});

test("textlint candidateはmain検査のCargo buildを再利用する", () => {
  validateBuildReuseContract({
    "tools/package-textlint-plugin-release.sh":
      'target="${ADOCWEAVE_TEXTLINT_PLUGIN_CARGO_TARGET_DIRECTORY:-target/textlint-wasm-build}"',
  });
  assert.throws(
    () => validateBuildReuseContract({
      "tools/package-textlint-plugin-release.sh":
        'target="${ADOCWEAVE_TEXTLINT_PLUGIN_CARGO_TARGET_DIRECTORY:-target/another-build}"',
    }),
    /同じCargo target directory/,
  );
});

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
  validateWritePermissionGrants({
    "release-publish.yml": {
      permissions: { contents: "read" },
      jobs: { publish: { permissions: { attestations: "write", contents: "write", "id-token": "write" } } },
    },
  });
  assert.throws(
    () => validateWritePermissionGrants({
      "release-publish.yml": {
        permissions: { contents: "read" },
        jobs: { publish: { permissions: {
          attestations: "write", contents: "write", "id-token": "write", issues: "write",
        } } },
      },
    }),
    /exactly the publication permissions/,
  );
});

test("the binary cache job may read the Cachix write token", () => {
  const { sources, workflows } = policyInput(
    "binary-cache-publish.yml",
    { jobs: { publish: { steps: [CACHE_STEP] } } },
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
    "binary-cache-publish.yml",
    { jobs: { publish: { steps: [OPEN_VSX_STEP] } } },
    OPEN_VSX_SOURCE,
  );
  assert.throws(
    () => validateNoDirectSecretAccess(sources, workflows),
    /job publish reads secrets\.OPEN_VSX_TOKEN/,
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
    "binary-cache-publish.yml",
    { jobs: { publish: { steps: [{ run: "echo ${{ secrets.OTHER_TOKEN }}" }] } } },
    "run: echo ${{ secrets.OTHER_TOKEN }}\n",
  );
  assert.throws(() => validateNoDirectSecretAccess(sources, workflows), /reads secrets\.OTHER_TOKEN/);
});

test("the Cachix token outside the binary cache job is rejected", () => {
  const { sources, workflows } = policyInput(
    "binary-cache-publish.yml",
    { jobs: { other: { steps: [CACHE_STEP] } } },
    CACHE_SOURCE,
  );
  assert.throws(
    () => validateNoDirectSecretAccess(sources, workflows),
    /job other reads secrets\.CACHIX_AUTH_TOKEN/,
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
    "binary-cache-publish.yml",
    {
      env: { CACHIX_AUTH_TOKEN: "${{ secrets.CACHIX_AUTH_TOKEN }}" },
      jobs: { publish: { steps: [{ run: "cachix push keishis result" }] } },
    },
    "env:\n  CACHIX_AUTH_TOKEN: ${{ secrets.CACHIX_AUTH_TOKEN }}\n",
  );
  assert.throws(() => validateNoDirectSecretAccess(sources, workflows), /outside an allowed job/);
});

function productRoutingWorkflows() {
  return {
    "release.yml": {
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
        "candidate-plan": {
          outputs: { product: "${{ steps.plan.outputs.product }}" },
        },
        "installation-e2e": {},
        "verify-global-candidate": {},
        "publish-native": {
          needs: ["candidate-plan", "installation-e2e"],
          uses: "./.github/workflows/release-publish.yml",
          with: { product: "${{ needs.candidate-plan.outputs.product }}" },
        },
        "publish-global": {
          needs: ["candidate-plan", "verify-global-candidate"],
          uses: "./.github/workflows/release-publish.yml",
          with: { product: "${{ needs.candidate-plan.outputs.product }}" },
        },
        "textlint-plugin-post-release-smoke": {
          if: "needs.candidate-plan.outputs.product == 'textlint'",
          needs: ["publish-global"],
        },
        "open-vsx": { if: "needs.candidate-plan.outputs.product == 'vscode'", needs: ["publish-global"] },
        "binary-cache": { if: "needs.candidate-plan.outputs.product == 'cli'", needs: ["publish-native"] },
      },
    },
    "release-publish.yml": {
      on: { workflow_call: { inputs: { product: { required: true, type: "string" } } } },
      jobs: {
        publish: {
          environment: "github-release",
          steps: [
            {
              uses: "actions/download-artifact@0000000000000000000000000000000000000000",
              with: { name: "release-candidate-${{ inputs.product }}" },
            },
            { name: "Immutable source tree verification", run: "git status --porcelain --untracked-files=all" },
            {
              name: "Immutable release input verification",
              run: 'node product-release --verify-publication "$PRODUCT"\nnode release-metadata verify "$PRODUCT" artifacts "$GITHUB_SHA"',
            },
            {
              uses: "actions/attest@1e69f48acb82d1966a394da916b4c1698aa569d6",
              with: { "subject-path": "artifacts/*" },
            },
            { name: "Immutable stable tag creation", run: 'actual_commit=x\ntest "$actual_commit" = "$GITHUB_SHA"' },
            { name: "Private draft creation and verification", run: "verify draft" },
            { name: "Complete release publication", run: "jq .tag_name\njq '.assets[].name'" },
            {
              name: "Incomplete draft removal",
              if: "failure() || cancelled()",
              run: 'adocweave-release-run\ntest "$(jq -r .draft <<<"$draft")" = true\ncontains($marker)\nDELETE releases',
            },
            {
              name: "Incomplete stable tag removal",
              if: "failure() || cancelled()",
              run: 'matching-refs/tags\nif [ -n "$existing" ]\nexpected="$RELEASE_TAG_SHA"\nif [ "$actual" != "$expected" ]\nGitHub Actions run $GITHUB_RUN_ID\ntest "$(jq -r .message <<<"$tag_object")" = "$expected_message"\ntest "$(jq -r .object.sha <<<"$tag_object")" = "$GITHUB_SHA"\nDELETE git/refs/tags/$RELEASE_TAG',
            },
          ],
        },
      },
    },
    "open-vsx-publish.yml": {
      jobs: { publish: { steps: [
        {
          name: "Published VSIX download and verification",
          run: "jq .draft\njq .prerelease\ngit rev-parse '$TAG^{commit}'\n--verify-candidate\nattestation verify",
        },
        { env: { TOKEN: "${{ secrets.OPEN_VSX_TOKEN }}" }, run: "publish" },
      ] } },
    },
    "binary-cache-publish.yml": {
      jobs: { publish: {
        strategy: { matrix: { include: [
          { nixSystem: "x86_64-linux" },
          { nixSystem: "aarch64-linux" },
        ] } },
        steps: [
          {
            name: "Published CLI candidate verification",
            run: "jq .draft\njq .prerelease\ngit rev-parse '$TAG^{commit}'\n--verify-candidate\nattestation verify",
          },
          {
            env: { TOKEN: "${{ secrets.CACHIX_AUTH_TOKEN }}" },
            run: 'check="$(nix build ".#checks.${NIX_SYSTEM}.default" --no-link --print-out-paths)"\npackage="$(readlink -f "$check/package")"\ncachix push keishis "$package"',
          },
        ],
      } },
    },
  };
}

test("product release routing accepts the separated product contracts", () => {
  validateProductReleaseRouting(productRoutingWorkflows());
});

test("product release routing rejects a post-release job shared by every product", () => {
  const workflows = productRoutingWorkflows();
  workflows["release.yml"].jobs["open-vsx"].if = "success()";
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

test("product release routing rejects publication before verification", () => {
  const workflows = productRoutingWorkflows();
  const steps = workflows["release-publish.yml"].jobs.publish.steps;
  [steps[2], steps[4]] = [steps[4], steps[2]];
  assert.throws(
    () => validateProductReleaseRouting(workflows),
    /verify, attest, tag, verify the draft, and publish in order/,
  );
});

test("product release routing requires the final tag and asset verification", () => {
  const workflows = productRoutingWorkflows();
  workflows["release-publish.yml"].jobs.publish.steps
    .find((step) => step.name === "Complete release publication").run = "publish";
  assert.throws(
    () => validateProductReleaseRouting(workflows),
    /tag commit and final release asset set/,
  );
});

test("release cleanup keeps another release or changed tag intact", () => {
  for (const remove of [
    "contains($marker)",
    'test "$(jq -r .draft <<<"$draft")" = true',
    'if [ -n "$existing" ]',
    'expected="$RELEASE_TAG_SHA"',
    'if [ "$actual" != "$expected" ]',
    'test "$(jq -r .message <<<"$tag_object")" = "$expected_message"',
    'test "$(jq -r .object.sha <<<"$tag_object")" = "$GITHUB_SHA"',
  ]) {
    const workflows = productRoutingWorkflows();
    for (const name of ["Incomplete draft removal", "Incomplete stable tag removal"]) {
      const step = workflows["release-publish.yml"].jobs.publish.steps.find((entry) => entry.name === name);
      step.run = step.run.replace(remove, "");
    }
    assert.throws(
      () => validateProductReleaseRouting(workflows),
      /remove only this run's draft and unchanged unpublished tag/,
    );
  }
});

test("product release routing attests every candidate file", () => {
  const workflows = productRoutingWorkflows();
  workflows["release-publish.yml"].jobs.publish.steps[3].with["subject-path"] = "plan.json";
  assert.throws(
    () => validateProductReleaseRouting(workflows),
    /attest every candidate file from a clean source tree/,
  );
});

test("post-release publication waits for GitHub Release and verifies stable state", () => {
  const missingDependency = productRoutingWorkflows();
  missingDependency["release.yml"].jobs["binary-cache"].needs = ["candidate-plan"];
  assert.throws(
    () => validateProductReleaseRouting(missingDependency),
    /binary-cache must wait for publish-native/,
  );

  const missingStableCheck = productRoutingWorkflows();
  missingStableCheck["open-vsx-publish.yml"].jobs.publish.steps[0].run = "publish";
  assert.throws(
    () => validateProductReleaseRouting(missingStableCheck),
    /accept only a published stable release/,
  );

  const wrongOrder = productRoutingWorkflows();
  wrongOrder["binary-cache-publish.yml"].jobs.publish.steps.reverse();
  assert.throws(
    () => validateProductReleaseRouting(wrongOrder),
    /accept only a published stable release/,
  );
});

test("binary cache publication checks the default package on both Linux architectures", () => {
  for (const system of ["x86_64-linux", "aarch64-linux"]) {
    const workflows = productRoutingWorkflows();
    const include = workflows["binary-cache-publish.yml"].jobs.publish.strategy.matrix.include;
    workflows["binary-cache-publish.yml"].jobs.publish.strategy.matrix.include =
      include.filter((entry) => entry.nixSystem !== system);
    assert.throws(
      () => validateProductReleaseRouting(workflows),
      /check and publish the default package for both Linux architectures/,
    );
  }

  for (const fragment of [
    'nix build ".#checks.${NIX_SYSTEM}.default"',
    'readlink -f "$check/package"',
    'cachix push keishis "$package"',
  ]) {
    const workflows = productRoutingWorkflows();
    const step = workflows["binary-cache-publish.yml"].jobs.publish.steps[1];
    step.run = step.run.replace(fragment, "removed");
    assert.throws(
      () => validateProductReleaseRouting(workflows),
      /check and publish the default package for both Linux architectures/,
    );
  }
});

function sourceAndCandidateWorkflow() {
  return {
    on: {
      pull_request: {},
      push: { branches: ["main"] },
      workflow_dispatch: { inputs: { product: { required: true, type: "choice" } } },
    },
    jobs: {
      source: {
        name: "verify",
        steps: [
          { if: "github.event_name == 'workflow_dispatch'", run: 'test "$GITHUB_REF" = refs/heads/main' },
          {
            run: 'nix develop .#ci -c bash -c \'command -v cargo\' | xargs dirname >> "$GITHUB_PATH"',
          },
          {
            uses: "Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6",
            with: {
              "prefix-key": "source",
              key: "${{ hashFiles('flake.lock', 'flake.nix', 'nix/**/*.nix') }}",
              workspaces: ". -> target\neditors/zed -> target\n",
              "cache-bin": "false",
              "cache-on-failure": "false",
              "save-if": "${{ github.event_name == 'push' && github.ref == 'refs/heads/main' }}",
            },
          },
          { run: "nix develop .#ci -c cargo make verify" },
        ],
      },
      security: {
        name: "security-audit",
        if: "(github.event_name == 'push' || github.event_name == 'workflow_dispatch') && github.ref == 'refs/heads/main'",
        needs: ["source"],
        steps: [{ run: "nix develop .#ci -c cargo make security-audit" }],
      },
      "main-gate": {
        if: "(github.event_name == 'push' || github.event_name == 'workflow_dispatch') && github.ref == 'refs/heads/main'",
        needs: ["source", "security"],
        steps: [{ run: "nix develop .#ci-fuzz -c cargo make main-gate" }],
      },
      "candidate-plan": {
        if: "github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/main'",
        needs: ["source", "main-gate"],
        steps: [{ run: 'node tools/product-candidate-plan.mjs "$GITHUB_OUTPUT" "${{ inputs.product }}"' }],
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
      "publish-native": {
        needs: ["candidate-plan", "installation-e2e"],
        uses: "./.github/workflows/release-publish.yml",
      },
      "publish-global": {
        needs: ["candidate-plan", "verify-global-candidate"],
        uses: "./.github/workflows/release-publish.yml",
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
    (workflow) => { workflow.jobs.source.steps[3].run += " || true"; },
    (workflow) => { delete workflow.jobs.source.steps[0].if; },
    (workflow) => { workflow.jobs["main-gate"].steps.push({ run: "cargo make verify" }); },
  ]) {
    const workflow = sourceAndCandidateWorkflow();
    mutate(workflow);
    assert.throws(() => validateSourceAndCandidateWorkflow(workflow), /source|必須check/);
  }

  const eventTypes = sourceAndCandidateWorkflow();
  eventTypes.on.pull_request.types = ["opened", "synchronize", "reopened"];
  validateSourceAndCandidateWorkflow(eventTypes);
});

test("sourceのRust cacheは固定環境を使い、main pushだけで保存する", () => {
  for (const mutate of [
    (workflow) => { workflow.jobs.source.steps.splice(1, 1); },
    (workflow) => { workflow.jobs.source.steps.splice(2, 1); },
    (workflow) => { workflow.jobs.source.steps[2].with["save-if"] = "true"; },
    (workflow) => { workflow.jobs.source.steps[2].with["cache-on-failure"] = "true"; },
    (workflow) => { workflow.jobs.source.steps[2].with.workspaces = ". -> target"; },
    (workflow) => { workflow.jobs.source.steps.push(workflow.jobs.source.steps.splice(2, 1)[0]); },
  ]) {
    const workflow = sourceAndCandidateWorkflow();
    mutate(workflow);
    assert.throws(() => validateSourceAndCandidateWorkflow(workflow), /cache/iu);
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

test("手動公開のcandidate planはsourceとmain gateの成功後に選択製品の計画を1回だけ生成する", () => {
  for (const mutate of [
    (workflow) => { workflow.jobs["candidate-plan"].needs = ["main-gate"]; },
    (workflow) => { workflow.jobs["candidate-plan"].steps = [{ run: "node custom-plan.mjs" }]; },
    (workflow) => { workflow.jobs["candidate-plan"].steps.push({ run: "node tools/product-candidate-plan.mjs" }); },
    (workflow) => { delete workflow.jobs["main-gate"].if; },
    (workflow) => { workflow.jobs["main-gate"].if += " || github.event_name == 'pull_request'"; },
    (workflow) => { workflow.jobs["main-gate"].if = `failure() && ${workflow.jobs["main-gate"].if}`; },
    (workflow) => { workflow.jobs["main-gate"].steps = []; },
    (workflow) => { workflow.jobs["main-gate"].steps[0].run += " || true"; },
    (workflow) => { workflow.jobs["main-gate"].needs = ["source"]; },
    (workflow) => { workflow.jobs.security.steps[0].run += " --offline"; },
    (workflow) => { workflow.jobs.security.if = "github.event_name == 'pull_request'"; },
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
    ["source dependency", (source) => mutateTask(source, "verify", (body) => body.replace('  "fmt-check",\n', ""))],
    ["main dependency", (source) => mutateTask(source, "main-gate", (body) => body.replace('  "fuzz-smoke",\n', ""))],
    ["main candidate", (source) => mutateTask(source, "main-gate", (body) => body.replace('  "nix-package-check",\n]', '  "nix-package-check",\n  "release-global-candidate",\n]'))],
    ["verify alias", (source) => mutateTask(source, "verify", (body) => `${body}\nalias = "main-gate"\n`)],
    ["acceptance", (source) => mutateTask(source, "acceptance", (body) => body.replace('  "main-gate",\n', ""))],
    ["release-check", (source) => mutateTask(source, "release-check", (body) => body.replace('  "security-audit",\n', ""))],
    ["public internal task", (source) => mutateTask(source, "clippy", (body) => body.replace("private = true\n", ""))],
    ["compatibility alias", (source) => `${source}\n[tasks.old-verify]\nalias = "verify"\n`],
    ["archive implementation", (source) => mutateTask(source, "acceptance", (body) => `${body}\nscript = '''\ntar -tf candidate.tar.xz\n'''\n`)],
    ["duplicated wasm compile", (source) => mutateTask(source, "build-wasm", (body) => `${body}\n# cargo build -p adocweave-wasm --release --target wasm32-unknown-unknown\n`)],
    ["duplicated workspace test", (source) => mutateTask(source, "test", (body) => `${body}\n# cargo test --workspace --all-features\n`)],
    ["three VSIX builds", (source) => mutateTask(source, "test-vscode-release-determinism", (body) => `${body}\nnpm run package --prefix editors/vscode\n`)],
  ]) {
    assert.throws(
      () => validateGateTaskContract(mutate(makefile)),
      undefined,
      `${name}の退行を拒否しませんでした`,
    );
  }
});
