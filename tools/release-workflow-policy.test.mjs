import assert from "node:assert/strict";
import { test } from "node:test";

import {
  validateBinaryCachePublication,
  validateCiGates,
  validateExternalPublicationIsolation,
  validateNpmPublication,
  validatePermissions,
  validatePinnedActions,
  validateReleaseFlow,
  validateReleaseVersionCommands,
} from "./release-workflow-policy.mjs";

const pin = "actions/checkout@0000000000000000000000000000000000000000";
const publicationTag = "${{ inputs.tag }}";
const publicationCommit = "${{ inputs.commit }}";

function releaseContractWorkflow() {
  return {
    on: { workflow_call: {} },
    permissions: { contents: "read" },
    jobs: {
      verify: {
        steps: [
          {
            uses: pin,
            with: { "fetch-depth": 0, "persist-credentials": false },
          },
          { run: "nix develop .#ci -c cargo make release-contract" },
          {
            if: "${{ github.event_name == 'push' }}",
            env: {
              RELEASE_COMMIT: "${{ github.sha }}",
              RELEASE_TAG: "${{ github.ref_name }}",
            },
            run: `
tag_commit="$(git rev-parse "refs/tags/$RELEASE_TAG^{commit}")"
test "$tag_commit" = "$RELEASE_COMMIT"
git merge-base --is-ancestor "$tag_commit" refs/remotes/origin/main
`,
          },
        ],
      },
    },
  };
}

function releasePublicationJob(workflow, oidc = false) {
  return {
    needs: ["plan", "host"],
    if: "${{ always() && needs.host.result == 'success' }}",
    uses: `./.github/workflows/${workflow}`,
    with: {
      tag: "${{ needs.plan.outputs.tag }}",
      commit: "${{ github.sha }}",
    },
    permissions: oidc
      ? { contents: "read", "id-token": "write" }
      : { contents: "read" },
  };
}

function releaseWorkflow() {
  return {
    on: { pull_request: {}, push: { tags: ["v[0-9]+.[0-9]+.[0-9]+"] } },
    permissions: { contents: "read" },
    jobs: {
      plan: {
        outputs: { publishing: "${{ !github.event.pull_request }}" },
        steps: [{ uses: pin }],
      },
      "custom-release-contract": { uses: "./.github/workflows/release-contract.yml" },
      "build-local-artifacts": { needs: ["plan", "custom-release-contract"], steps: [] },
      "build-global-artifacts": { needs: ["plan", "build-local-artifacts"], steps: [] },
      "custom-native-artifact-smoke": { needs: ["plan", "build-local-artifacts"], steps: [] },
      host: {
        needs: [
          "plan",
          "custom-release-contract",
          "build-local-artifacts",
          "build-global-artifacts",
          "custom-native-artifact-smoke",
        ],
        if: "${{ always() && needs.plan.result == 'success' && needs.custom-release-contract.result == 'success' && needs.build-local-artifacts.result == 'success' && needs.build-global-artifacts.result == 'success' && needs.custom-native-artifact-smoke.result == 'success' && needs.plan.outputs.publishing == 'true' }}",
        environment: "github-release",
        permissions: { attestations: "write", contents: "write", "id-token": "write" },
        steps: [
          {
            uses: "actions/attest-build-provenance@0000000000000000000000000000000000000000",
            with: { "subject-path": "artifacts/*" },
          },
        ],
      },
      "publish-binary-cache": releasePublicationJob("binary-cache-publish.yml"),
      "publish-marketplace": releasePublicationJob("marketplace-publish.yml", true),
      "publish-npm": releasePublicationJob("npm-publish.yml", true),
      "publish-open-vsx": releasePublicationJob("open-vsx-publish.yml"),
      announce: {
        needs: [
          "plan",
          "host",
          "publish-binary-cache",
          "publish-marketplace",
          "publish-npm",
          "publish-open-vsx",
        ],
        if: "${{ always() && needs.host.result == 'success' && needs.publish-binary-cache.result == 'success' && needs.publish-marketplace.result == 'success' && needs.publish-npm.result == 'success' && needs.publish-open-vsx.result == 'success' }}",
        steps: [],
      },
    },
  };
}

function ciWorkflow() {
  const main = "${{ github.event_name == 'push' && github.ref == 'refs/heads/main' }}";
  return {
    on: { pull_request: {}, push: { branches: ["main"] } },
    permissions: { actions: "read", contents: "read" },
    jobs: {
      source: { steps: [{ uses: pin }, { run: "cargo make verify" }] },
      security: { if: main, needs: ["source"], steps: [] },
      "main-integrations": { if: main, needs: ["source", "security"], steps: [] },
      "fuzz-smoke": { if: main, needs: ["source", "security"], steps: [] },
      "nix-package-check": { if: main, needs: ["source", "security"], steps: [] },
    },
  };
}

const stableReleaseVerification = `
gh api "repos/$GITHUB_REPOSITORY/releases/tags/$RELEASE_TAG" | jq .draft,.prerelease
git rev-parse "refs/tags/$RELEASE_TAG^{commit}"
git rev-parse HEAD
workspace_version=1.2.3
test "$RELEASE_TAG" = "v$workspace_version"
`;

function publicationWorkflow(environment, oidc = false) {
  return {
    on: {
      workflow_call: {
        inputs: {
          tag: { required: true, type: "string" },
          commit: { required: true, type: "string" },
        },
      },
    },
    permissions: { contents: "read" },
    concurrency: { group: `publication-${publicationTag}`, "cancel-in-progress": false },
    jobs: {
      publish: {
        environment,
        ...(oidc ? { permissions: { contents: "read", "id-token": "write" } } : {}),
        env: { RELEASE_TAG: publicationTag, RELEASE_COMMIT: publicationCommit },
        steps: [
          { uses: pin, with: { ref: publicationCommit } },
          { run: stableReleaseVerification },
        ],
      },
    },
  };
}

function npmPublicationWorkflow() {
  const workflow = publicationWorkflow("npm-publish", true);
  workflow.jobs.publish.environment = "npm-publish";
  workflow.jobs.publish.strategy = {
    "fail-fast": false,
    matrix: { package: ["textlint", "wasm"] },
  };
  workflow.jobs.publish.steps.push({
    env: { RELEASE_PACKAGE: "${{ matrix.package }}" },
    run: "test $RELEASE_COMMIT = ${{ inputs.commit }}\nnode tools/npm-publication.mjs",
  });
  return workflow;
}

function binaryCachePublicationWorkflow() {
  const matrix = {
    "fail-fast": false,
    matrix: {
      include: [
        { runner: "ubuntu-24.04", nixSystem: "x86_64-linux" },
        { runner: "ubuntu-24.04-arm", nixSystem: "aarch64-linux" },
      ],
    },
  };
  return {
    on: {
      workflow_call: {
        inputs: {
          tag: { required: true, type: "string" },
          commit: { required: true, type: "string" },
        },
      },
    },
    permissions: { contents: "read" },
    concurrency: { group: `binary-cache-${publicationTag}`, "cancel-in-progress": false },
    jobs: {
      publish: {
        environment: "binary-cache-publish",
        env: { RELEASE_TAG: publicationTag, RELEASE_COMMIT: publicationCommit },
        strategy: matrix,
        steps: [
          {
            uses: pin,
            with: { ref: publicationCommit },
          },
          {
            run: `${stableReleaseVerification}
cachix push keishis "$package"
`,
          },
          {
            uses: "cachix/cachix-action@0000000000000000000000000000000000000000",
            with: {
              name: "keishis",
              authToken: "${{ secrets.CACHIX_AUTH_TOKEN }}",
              skipPush: true,
            },
          },
        ],
      },
      verify: {
        needs: "publish",
        strategy: structuredClone(matrix),
        steps: [
          { uses: pin, with: { ref: publicationCommit } },
          {
            uses: "cachix/cachix-action@0000000000000000000000000000000000000000",
            with: { name: "keishis", skipPush: true },
          },
          {
            run: `
nix build ".#packages.\${NIX_SYSTEM}.default" \\
  --option builders '' \\
  --option fallback false \\
  --option max-jobs 0 \\
  --option substituters https://keishis.cachix.org
node tools/binary-cache-smoke.mjs "$package/bin/adocweave"
`,
          },
        ],
      },
    },
  };
}

function marketplacePublicationWorkflow() {
  const workflow = publicationWorkflow("marketplace-publish", true);
  workflow.jobs.publish.steps.push({
    run: "npx vsce publish --packagePath extension.vsix --oidc",
  });
  return workflow;
}

function openVsxPublicationWorkflow() {
  const workflow = publicationWorkflow("open-vsx-publish");
  workflow.jobs.publish.steps.push({
    env: { OPEN_VSX_TOKEN: "${{ secrets.OPEN_VSX_TOKEN }}" },
    run: "curl --url-query token=$OPEN_VSX_TOKEN https://open-vsx.org/api/-/publish",
  });
  return workflow;
}

function workflows() {
  return {
    "release.yml": releaseWorkflow(),
    "release-contract.yml": releaseContractWorkflow(),
    "ci.yml": ciWorkflow(),
    "native-artifact-smoke.yml": {
      on: { workflow_call: {} },
      permissions: { contents: "read" },
      jobs: { smoke: { steps: [] } },
    },
    "binary-cache-publish.yml": binaryCachePublicationWorkflow(),
    "marketplace-publish.yml": marketplacePublicationWorkflow(),
    "npm-publish.yml": npmPublicationWorkflow(),
    "open-vsx-publish.yml": openVsxPublicationWorkflow(),
  };
}

function releaseFlowWorkflows(release) {
  return {
    "release.yml": release,
    "release-contract.yml": releaseContractWorkflow(),
  };
}

test("すべての外部Actionを完全なcommit SHAへ固定する", () => {
  const fixtures = workflows();
  validatePinnedActions(fixtures);
  fixtures["ci.yml"].jobs.source.steps[0].uses = "actions/checkout@v7";
  assert.throws(() => validatePinnedActions(fixtures), /not pinned to a full commit SHA/);
});

test("workflowと公開jobの権限を必要最小限に限定する", () => {
  const fixtures = workflows();
  validatePermissions(fixtures);

  fixtures["ci.yml"].permissions["id-token"] = "write";
  assert.throws(() => validatePermissions(fixtures), /ci\.yml top-level permissions/);
  delete fixtures["ci.yml"].permissions["id-token"];

  fixtures["release.yml"].permissions.contents = "write";
  assert.throws(() => validatePermissions(fixtures), /release\.yml top-level permissions/);
  fixtures["release.yml"].permissions.contents = "read";

  fixtures["release.yml"].jobs.host.permissions.actions = "write";
  assert.throws(() => validatePermissions(fixtures), /release\.yml host permissions/);
  delete fixtures["release.yml"].jobs.host.permissions.actions;

  fixtures["release.yml"].jobs["publish-npm"].permissions["id-token"] = "read";
  assert.throws(() => validatePermissions(fixtures), /release\.yml publish-npm permissions/);
  fixtures["release.yml"].jobs["publish-npm"].permissions["id-token"] = "write";

  fixtures["open-vsx-publish.yml"].jobs.publish.permissions = { contents: "write" };
  assert.throws(() => validatePermissions(fixtures), /unexpected contents: write/);
});

test("ReleaseはPRでplanだけを作り、tagの成功済み成果物だけをhostする", () => {
  const release = releaseWorkflow();
  validateReleaseFlow(releaseFlowWorkflows(release), 'pr-run-mode = "plan"');

  release.on.push = { branches: ["main"] };
  assert.throws(
    () => validateReleaseFlow(releaseFlowWorkflows(release), 'pr-run-mode = "plan"'),
    /tag.*only/,
  );
});

test("共通contractはstable tagがmainに含まれることをhost前に一度だけ検査する", () => {
  const fixtures = releaseFlowWorkflows(releaseWorkflow());
  validateReleaseFlow(fixtures, 'pr-run-mode = "plan"');

  fixtures["release-contract.yml"].jobs.verify.steps[0].with["fetch-depth"] = 1;
  assert.throws(
    () => validateReleaseFlow(fixtures, 'pr-run-mode = "plan"'),
    /fetch complete history/u,
  );

  const late = releaseFlowWorkflows(releaseWorkflow());
  late["release-contract.yml"].jobs.verify.steps.reverse();
  assert.throws(
    () => validateReleaseFlow(late, 'pr-run-mode = "plan"'),
    /main ancestry once after tag and version checks/u,
  );

  const bypassed = releaseFlowWorkflows(releaseWorkflow());
  bypassed["release.yml"].jobs.host.needs = bypassed["release.yml"].jobs.host.needs.filter(
    (job) => job !== "custom-release-contract",
  );
  assert.throws(
    () => validateReleaseFlow(bypassed, 'pr-run-mode = "plan"'),
    /host must wait for.*common contract/u,
  );
});

test("hostはplan、local、global、native smokeの成功をすべて要求する", () => {
  const release = releaseWorkflow();
  release.jobs.host.if = release.jobs.host.if.replace(
    "needs.build-global-artifacts.result == 'success'",
    "needs.build-global-artifacts.result == 'skipped'",
  );
  assert.throws(
    () => validateReleaseFlow(releaseFlowWorkflows(release), 'pr-run-mode = "plan"'),
    /build-global-artifacts success/,
  );
});

test("hostの書込み権限とOIDCをgithub-release environmentへ隔離する", () => {
  const release = releaseWorkflow();
  delete release.jobs.host.environment;
  assert.throws(
    () => validateReleaseFlow(releaseFlowWorkflows(release), 'pr-run-mode = "plan"'),
    /github-release environment/u,
  );
});

test("hostは公開する全成果物をattestation対象にする", () => {
  const release = releaseWorkflow();
  release.jobs.host.steps[0].with["subject-path"] = "artifacts/*.zip";
  assert.throws(
    () => validateReleaseFlow(releaseFlowWorkflows(release), 'pr-run-mode = "plan"'),
    /attest every published artifact/,
  );
});

test("Release成功後は4つの公開workflowを直接呼び出して成否を集約する", () => {
  const release = releaseWorkflow();
  validateReleaseFlow(releaseFlowWorkflows(release), 'pr-run-mode = "plan"');

  release.jobs["publish-npm"].uses = "./.github/workflows/open-vsx-publish.yml";
  assert.throws(
    () => validateReleaseFlow(releaseFlowWorkflows(release), 'pr-run-mode = "plan"'),
    /publish-npm must call npm-publish\.yml/u,
  );

  const incomplete = releaseWorkflow();
  incomplete.jobs.announce.needs = incomplete.jobs.announce.needs.filter(
    (job) => job !== "publish-marketplace",
  );
  assert.throws(
    () => validateReleaseFlow(releaseFlowWorkflows(incomplete), 'pr-run-mode = "plan"'),
    /announce must wait for every external publication/u,
  );

  const skipped = releaseWorkflow();
  skipped.jobs.announce.if = skipped.jobs.announce.if.replace(
    "needs.publish-open-vsx.result == 'success'",
    "(needs.publish-open-vsx.result == 'success' || needs.publish-open-vsx.result == 'skipped')",
  );
  assert.throws(
    () => validateReleaseFlow(releaseFlowWorkflows(skipped), 'pr-run-mode = "plan"'),
    /must not report success after a skipped publication/u,
  );
});

test("CIはPRのsource gateとmain専用gateを分離する", () => {
  const ci = ciWorkflow();
  validateCiGates({ "ci.yml": ci });
  delete ci.jobs["main-integrations"].if;
  assert.throws(() => validateCiGates({ "ci.yml": ci }), /main-integrations.*main-only/);
});

test("外部公開workflowはReleaseからの再利用呼出しだけを受け付ける", () => {
  const fixtures = workflows();
  validateExternalPublicationIsolation(fixtures);
  fixtures["npm-publish.yml"].on.workflow_dispatch = {};
  assert.throws(
    () => validateExternalPublicationIsolation(fixtures),
    /npm-publish.*callable only/u,
  );

  const called = workflows();
  called["open-vsx-publish.yml"].on.workflow_call.inputs.commit.required = false;
  assert.throws(
    () => validateExternalPublicationIsolation(called),
    /open-vsx-publish.*stable tag and complete commit/u,
  );

  const duplicateValidation = workflows();
  duplicateValidation["binary-cache-publish.yml"].jobs.validate = { steps: [] };
  assert.throws(
    () => validateExternalPublicationIsolation(duplicateValidation),
    /binary-cache-publish.*isolated publication jobs/u,
  );

  const duplicateAncestry = workflows();
  duplicateAncestry["npm-publish.yml"].jobs.publish.steps.push({
    run: "git merge-base --is-ancestor HEAD refs/remotes/origin/main",
  });
  assert.throws(
    () => validateExternalPublicationIsolation(duplicateAncestry),
    /npm-publish.*leave main ancestry verification.*common release contract/u,
  );

  const extraSender = workflows();
  extraSender["open-vsx-publish.yml"].jobs.publish.steps.push({
    run: 'gh api "repos/$GITHUB_REPOSITORY/dispatches"',
  });
  assert.throws(
    () => validateExternalPublicationIsolation(extraSender),
    /must not use dispatch events/u,
  );
});

test("Open VSXのtokenを専用workflowの公開job以外へ渡さない", () => {
  const fixtures = workflows();
  validateExternalPublicationIsolation(fixtures);

  fixtures["binary-cache-publish.yml"].jobs.verify.steps.push({
    env: { TOKEN: "${{ secrets.OPEN_VSX_TOKEN }}" },
    run: "true",
  });
  assert.throws(
    () => validateExternalPublicationIsolation(fixtures),
    /OPEN_VSX_TOKEN.*only by open-vsx-publish\.yml publish/u,
  );
});

test("Marketplace公開は専用environmentとOIDCだけを使う", () => {
  const fixtures = workflows();
  validateExternalPublicationIsolation(fixtures);

  const marketplaceStep = fixtures["marketplace-publish.yml"].jobs.publish.steps.find((step) =>
    String(step.run ?? "").includes("vsce publish")
  );
  marketplaceStep.run =
    "npx vsce publish --packagePath extension.vsix --azure-credential";
  assert.throws(
    () => validateExternalPublicationIsolation(fixtures),
    /trusted publishing with OIDC/u,
  );

  const azure = workflows();
  azure["marketplace-publish.yml"].jobs.publish.steps.push({
    uses: "Azure/login@0000000000000000000000000000000000000000",
  });
  assert.throws(
    () => validateExternalPublicationIsolation(azure),
    /trusted publishing with OIDC/u,
  );

  const shared = workflows();
  shared["npm-publish.yml"].jobs.publish.environment = "marketplace-publish";
  assert.throws(
    () => validateExternalPublicationIsolation(shared),
    /credentials must stay in the npm-publish environment/u,
  );
});

test("Cachix公開は二つのLinux closureを送り別のtokenなしrunnerで取得する", () => {
  const fixtures = workflows();
  validateBinaryCachePublication(fixtures);

  fixtures["binary-cache-publish.yml"].jobs.verify.steps[2].run =
    fixtures["binary-cache-publish.yml"].jobs.verify.steps[2].run.replace(
      "--option max-jobs 0",
      "--option max-jobs 1",
    );
  assert.throws(
    () => validateBinaryCachePublication(fixtures),
    /acquire and smoke.*max-jobs/u,
  );
});

test("Cachixの書込みtokenを公開job以外へ渡さない", () => {
  const fixtures = workflows();
  fixtures["binary-cache-publish.yml"].jobs.verify.steps[1].with.authToken =
    "${{ secrets.CACHIX_AUTH_TOKEN }}";
  assert.throws(
    () => validateBinaryCachePublication(fixtures),
    /without a write token/u,
  );

  const separateFixtures = workflows();
  separateFixtures["open-vsx-publish.yml"].jobs.publish.steps.push({
    env: { TOKEN: "${{ secrets.CACHIX_AUTH_TOKEN }}" },
    run: "true",
  });
  assert.throws(
    () => validateBinaryCachePublication(separateFixtures),
    /used only by binary-cache-publish/u,
  );
});

test("npm公開は利用者にpackageを選ばせず二つの固定対象を検証する", () => {
  const fixtures = workflows();
  validateNpmPublication(fixtures);
  fixtures["npm-publish.yml"].on.workflow_call.inputs.package = {};
  assert.throws(() => validateNpmPublication(fixtures), /accept only the release tag and commit/);
  delete fixtures["npm-publish.yml"].on.workflow_call.inputs.package;
  fixtures["npm-publish.yml"].jobs.publish.strategy.matrix.package = ["wasm"];
  assert.throws(() => validateNpmPublication(fixtures), /both fixed packages/);
});

test("release guideは単一versionの--checkと--versionだけを使う", () => {
  validateReleaseVersionCommands(`
node tools/sync-release-version.mjs --version X.Y.Z
node tools/sync-release-version.mjs --check
`);
  assert.throws(
    () => validateReleaseVersionCommands(
      "node tools/sync-release-version.mjs --product cli --version X.Y.Z",
    ),
    /使用方法/,
  );
});
