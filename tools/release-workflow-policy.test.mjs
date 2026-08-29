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
const publicationTag =
  "${{ github.event_name == 'repository_dispatch' && github.event.client_payload.tag || inputs.tag }}";

function releaseWorkflow() {
  return {
    on: { pull_request: {}, push: { tags: ["v[0-9]+.[0-9]+.[0-9]+"] } },
    permissions: { contents: "read" },
    jobs: {
      plan: {
        outputs: { publishing: "${{ !github.event.pull_request }}" },
        steps: [{ uses: pin }],
      },
      "build-local-artifacts": { needs: ["plan"], steps: [] },
      "build-global-artifacts": { needs: ["plan", "build-local-artifacts"], steps: [] },
      "custom-native-artifact-smoke": { needs: ["plan", "build-local-artifacts"], steps: [] },
      host: {
        needs: [
          "plan",
          "build-local-artifacts",
          "build-global-artifacts",
          "custom-native-artifact-smoke",
        ],
        if: "${{ always() && needs.plan.result == 'success' && needs.build-local-artifacts.result == 'success' && needs.build-global-artifacts.result == 'success' && needs.custom-native-artifact-smoke.result == 'success' && needs.plan.outputs.publishing == 'true' }}",
        permissions: { attestations: "write", contents: "write", "id-token": "write" },
        steps: [
          {
            uses: "actions/attest-build-provenance@0000000000000000000000000000000000000000",
            with: { "subject-path": "artifacts/*" },
          },
        ],
      },
      "dispatch-publication": {
        needs: ["plan", "host"],
        if: "${{ always() && needs.host.result == 'success' }}",
        permissions: { contents: "write" },
        env: {
          GH_TOKEN: "${{ github.token }}",
          RELEASE_TAG: "${{ needs.plan.outputs.tag }}",
        },
        steps: [{
          run: `
gh api "repos/$GITHUB_REPOSITORY/releases/tags/$RELEASE_TAG" | jq .draft,.prerelease
gh api --method POST "repos/$GITHUB_REPOSITORY/dispatches" \\
  --field event_type=adocweave_release_published \\
  --field "client_payload[tag]=$RELEASE_TAG"
`,
        }],
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

function validationJob() {
  return {
    outputs: {
      tag: "${{ steps.input.outputs.tag }}",
      commit: "${{ steps.candidate.outputs.commit }}",
    },
    steps: [
      {
        id: "input",
        env: { REQUESTED_TAG: publicationTag },
        run: `
[[ "$REQUESTED_TAG" =~ ^v(0|[1-9][0-9]*) ]]
gh api "repos/$GITHUB_REPOSITORY/releases/tags/$REQUESTED_TAG" | jq .draft,.prerelease
gh api "repos/$GITHUB_REPOSITORY/git/ref/tags/$REQUESTED_TAG"
`,
      },
      {
        uses: pin,
        with: {
          ref: "refs/tags/${{ steps.input.outputs.tag }}",
          "persist-credentials": false,
        },
      },
      {
        id: "candidate",
        run: `
git rev-parse "refs/tags/$RELEASE_TAG^{commit}"
git merge-base --is-ancestor "$commit" refs/remotes/origin/main
workspace_version=1.2.3
test "$RELEASE_TAG" = "v$workspace_version"
`,
      },
    ],
  };
}

function publicationWorkflow(environment, oidc = false) {
  return {
    on: {
      repository_dispatch: { types: ["adocweave_release_published"] },
      workflow_dispatch: { inputs: { tag: { required: true, type: "string" } } },
    },
    permissions: { contents: "read" },
    concurrency: { group: `publication-${publicationTag}`, "cancel-in-progress": false },
    jobs: {
      validate: validationJob(),
      publish: {
        needs: "validate",
        environment,
        ...(oidc ? { permissions: { contents: "read", "id-token": "write" } } : {}),
        env: { RELEASE_COMMIT: "${{ needs.validate.outputs.commit }}" },
        steps: [{
          uses: pin,
          with: { ref: "${{ needs.validate.outputs.commit }}" },
        }],
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
  workflow.jobs.publish.steps = [{
    uses: pin,
    with: { ref: "${{ needs.validate.outputs.commit }}" },
  }, {
    env: { RELEASE_PACKAGE: "${{ matrix.package }}" },
    run: "test $RELEASE_COMMIT = ${{ needs.validate.outputs.commit }}\nnode tools/npm-publication.mjs",
  }];
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
      repository_dispatch: { types: ["adocweave_release_published"] },
      workflow_dispatch: { inputs: { tag: { required: true, type: "string" } } },
    },
    permissions: { contents: "read" },
    concurrency: { group: `binary-cache-${publicationTag}`, "cancel-in-progress": false },
    jobs: {
      validate: validationJob(),
      publish: {
        needs: "validate",
        environment: "binary-cache-publish",
        env: { RELEASE_TAG: publicationTag },
        strategy: matrix,
        steps: [
          {
            uses: pin,
            with: { ref: "${{ needs.validate.outputs.commit }}" },
          },
          {
            run: `
gh api "repos/$GITHUB_REPOSITORY/releases/tags/$RELEASE_TAG" | jq .draft,.prerelease
git rev-parse "refs/tags/$RELEASE_TAG^{commit}"
git rev-parse HEAD
workspace_version=1.2.3
test "$RELEASE_TAG" = "v$workspace_version"
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
        needs: ["validate", "publish"],
        strategy: structuredClone(matrix),
        steps: [
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

function workflows() {
  return {
    "release.yml": releaseWorkflow(),
    "ci.yml": ciWorkflow(),
    "native-artifact-smoke.yml": {
      on: { workflow_call: {} },
      permissions: { contents: "read" },
      jobs: { smoke: { steps: [] } },
    },
    "binary-cache-publish.yml": binaryCachePublicationWorkflow(),
    "marketplace-publish.yml": {
      on: {
        repository_dispatch: { types: ["adocweave_release_published"] },
        workflow_dispatch: { inputs: { tag: { required: true, type: "string" } } },
      },
      permissions: { contents: "read" },
      concurrency: { group: `marketplace-${publicationTag}`, "cancel-in-progress": false },
      jobs: {
        validate: validationJob(),
        publish: {
          needs: "validate",
          environment: "marketplace-publish",
          permissions: { contents: "read", "id-token": "write" },
          env: { RELEASE_COMMIT: "${{ needs.validate.outputs.commit }}" },
          steps: [
            { uses: pin, with: { ref: "${{ needs.validate.outputs.commit }}" } },
            { run: "npx vsce publish --packagePath extension.vsix --oidc" },
          ],
        },
      },
    },
    "npm-publish.yml": npmPublicationWorkflow(),
    "open-vsx-publish.yml": publicationWorkflow("open-vsx-publish"),
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

  fixtures["open-vsx-publish.yml"].jobs.publish.permissions = { contents: "write" };
  assert.throws(() => validatePermissions(fixtures), /unexpected contents: write/);
});

test("ReleaseはPRでplanだけを作り、tagの成功済み成果物だけをhostする", () => {
  const release = releaseWorkflow();
  validateReleaseFlow({ "release.yml": release }, 'pr-run-mode = "plan"');

  release.on.push = { branches: ["main"] };
  assert.throws(
    () => validateReleaseFlow({ "release.yml": release }, 'pr-run-mode = "plan"'),
    /tag.*only/,
  );
});

test("hostはplan、local、global、native smokeの成功をすべて要求する", () => {
  const release = releaseWorkflow();
  release.jobs.host.if = release.jobs.host.if.replace(
    "needs.build-global-artifacts.result == 'success'",
    "needs.build-global-artifacts.result == 'skipped'",
  );
  assert.throws(
    () => validateReleaseFlow({ "release.yml": release }, 'pr-run-mode = "plan"'),
    /build-global-artifacts success/,
  );
});

test("hostは公開する全成果物をattestation対象にする", () => {
  const release = releaseWorkflow();
  release.jobs.host.steps[0].with["subject-path"] = "artifacts/*.zip";
  assert.throws(
    () => validateReleaseFlow({ "release.yml": release }, 'pr-run-mode = "plan"'),
    /attest every published artifact/,
  );
});

test("Release成功後はtagだけを一つのrepository dispatchで通知する", () => {
  const release = releaseWorkflow();
  validateReleaseFlow({ "release.yml": release }, 'pr-run-mode = "plan"');

  release.jobs["dispatch-publication"].steps[0].run +=
    "\ngh workflow run npm-publish.yml -f tag=$RELEASE_TAG";
  assert.throws(
    () => validateReleaseFlow({ "release.yml": release }, 'pr-run-mode = "plan"'),
    /must not enumerate workflows/u,
  );
});

test("CIはPRのsource gateとmain専用gateを分離する", () => {
  const ci = ciWorkflow();
  validateCiGates({ "ci.yml": ci });
  delete ci.jobs["main-integrations"].if;
  assert.throws(() => validateCiGates({ "ci.yml": ci }), /main-integrations.*main-only/);
});

test("外部公開workflowは共通通知とtagだけの手動入口を持つ", () => {
  const fixtures = workflows();
  validateExternalPublicationIsolation(fixtures);
  fixtures["npm-publish.yml"].on.push = { tags: ["v*"] };
  assert.throws(
    () => validateExternalPublicationIsolation(fixtures),
    /npm-publish.*shared repository dispatch/u,
  );

  const called = workflows();
  called["open-vsx-publish.yml"].on.workflow_call = {};
  assert.throws(
    () => validateExternalPublicationIsolation(called),
    /open-vsx-publish.*shared repository dispatch/u,
  );

  const optional = workflows();
  optional["binary-cache-publish.yml"].on.workflow_dispatch.inputs.tag.required = false;
  assert.throws(
    () => validateExternalPublicationIsolation(optional),
    /binary-cache-publish.*manual recovery/u,
  );

  const extraSender = workflows();
  extraSender["open-vsx-publish.yml"].jobs.publish.steps.push({
    run: 'gh api "repos/$GITHUB_REPOSITORY/dispatches"',
  });
  assert.throws(
    () => validateExternalPublicationIsolation(extraSender),
    /sent once and only by the Release dispatch job/u,
  );
});

test("Marketplace公開は専用environmentとOIDCだけを使う", () => {
  const fixtures = workflows();
  validateExternalPublicationIsolation(fixtures);

  fixtures["marketplace-publish.yml"].jobs.publish.steps[0].run =
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

  fixtures["binary-cache-publish.yml"].jobs.verify.steps[1].run =
    fixtures["binary-cache-publish.yml"].jobs.verify.steps[1].run.replace(
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
  fixtures["binary-cache-publish.yml"].jobs.verify.steps[0].with.authToken =
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
  fixtures["npm-publish.yml"].on.workflow_dispatch.inputs.package = {};
  assert.throws(() => validateNpmPublication(fixtures), /accept only the release tag/);
  delete fixtures["npm-publish.yml"].on.workflow_dispatch.inputs.package;
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
