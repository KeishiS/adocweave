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

function publicationWorkflow(oidc = false) {
  return {
    on: { workflow_call: {}, workflow_dispatch: {} },
    permissions: { contents: "read" },
    jobs: { publish: { ...(oidc ? { permissions: { "id-token": "write" } } : {}), steps: [] } },
  };
}

function npmPublicationWorkflow() {
  const workflow = publicationWorkflow(true);
  workflow.on.workflow_call.inputs = { tag: {} };
  workflow.on.workflow_dispatch.inputs = { tag: {} };
  workflow.jobs.publish.environment = "npm-publish";
  workflow.jobs.publish.strategy = {
    "fail-fast": false,
    matrix: { package: ["textlint", "wasm"] },
  };
  workflow.jobs.publish.steps = [{
    env: { RELEASE_PACKAGE: "${{ matrix.package }}" },
    run: "node tools/npm-publication.mjs",
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
    on: { workflow_call: {}, workflow_dispatch: {} },
    permissions: { contents: "read" },
    jobs: {
      publish: {
        strategy: matrix,
        steps: [
          {
            run: `
gh api "repos/$GITHUB_REPOSITORY/releases/tags/$RELEASE_TAG" | jq .draft,.prerelease
git rev-parse "$RELEASE_TAG^{commit}"
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
        needs: "publish",
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
      on: { workflow_call: {}, workflow_dispatch: {} },
      permissions: { contents: "read" },
      jobs: {
        publish: {
          environment: "marketplace-publish",
          permissions: { contents: "read", "id-token": "write" },
          steps: [{ run: "npx vsce publish --packagePath extension.vsix --oidc" }],
        },
      },
    },
    "npm-publish.yml": npmPublicationWorkflow(),
    "open-vsx-publish.yml": publicationWorkflow(),
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

test("CIはPRのsource gateとmain専用gateを分離する", () => {
  const ci = ciWorkflow();
  validateCiGates({ "ci.yml": ci });
  delete ci.jobs["main-integrations"].if;
  assert.throws(() => validateCiGates({ "ci.yml": ci }), /main-integrations.*main-only/);
});

test("外部公開workflowは個別の再利用・手動入口だけを持つ", () => {
  const fixtures = workflows();
  validateExternalPublicationIsolation(fixtures);
  fixtures["npm-publish.yml"].on.push = { tags: ["v*"] };
  assert.throws(
    () => validateExternalPublicationIsolation(fixtures),
    /npm-publish.*isolated/,
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
    /isolated to marketplace-publish/u,
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
