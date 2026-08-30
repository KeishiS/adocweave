import assert from "node:assert/strict";
import { test } from "node:test";

import {
  validateCachixPublication,
  validateCiGates,
  validateExternalPublicationIsolation,
  validatePermissions,
  validatePinnedActions,
  validateReleaseFlow,
  validateNativeVersionCommands,
  validateTextlintPluginPublication,
  validateVscodePublication,
  validateWasmPublication,
} from "./workflow-policy-library.mjs";

const pin = "actions/checkout@0000000000000000000000000000000000000000";
const publicationTag = "${{ inputs.tag }}";
const publicationCommit = "${{ inputs.commit }}";
const wasmNpmSmoke = `
const args = ["audit", "signatures", "--include-attestations"];
runWasmPackageBrowserSmoke(packageRoot);
`;
const textlintNpmSmoke = `
const args = ["audit", "signatures", "--include-attestations"];
runTextlintPluginConsumerE2E(spec);
runTextlintPluginNpxSmoke(spec);
`;

function nativeReleaseChecksWorkflow() {
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
          { run: "nix develop .#ci -c cargo make native-release-checks" },
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
      "custom-native-release-checks": { uses: "./.github/workflows/native-release-checks.yml" },
      "build-local-artifacts": { needs: ["plan", "custom-native-release-checks"], steps: [] },
      "build-global-artifacts": { needs: ["plan", "build-local-artifacts"], steps: [] },
      "custom-native-artifact-smoke": {
        needs: ["plan", "build-local-artifacts"],
        uses: "./.github/workflows/native-artifact-smoke.yml",
      },
      host: {
        needs: [
          "plan",
          "custom-native-release-checks",
          "build-local-artifacts",
          "build-global-artifacts",
          "custom-native-artifact-smoke",
        ],
        if: "${{ always() && needs.plan.result == 'success' && needs.custom-native-release-checks.result == 'success' && needs.build-local-artifacts.result == 'success' && needs.build-global-artifacts.result == 'success' && needs.custom-native-artifact-smoke.result == 'success' && needs.plan.outputs.publishing == 'true' }}",
        environment: "github-release",
        permissions: { attestations: "write", contents: "write", "id-token": "write" },
        steps: [
          {
            uses: "actions/attest-build-provenance@0000000000000000000000000000000000000000",
            with: { "subject-path": "artifacts/*" },
          },
          {
            run: `
rm -f artifacts/*-dist-manifest.json
gh release create v1.2.3 artifacts/*
`,
          },
        ],
      },
      "publish-cachix": releasePublicationJob("cachix-publish.yml"),
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

function textlintPluginPublicationWorkflow() {
  return {
    on: {
      push: { tags: ["textlint-plugin-asciidoc/v[0-9]+.[0-9]+.[0-9]+"] },
    },
    permissions: { contents: "read" },
    concurrency: {
      group: "textlint-plugin-publish-${{ github.ref_name }}",
      "cancel-in-progress": false,
    },
    jobs: {
      candidate: {
        steps: [
          {
            uses: pin,
            with: { "fetch-depth": 0, "fetch-tags": true, "persist-credentials": false },
          },
          {
            run: `
version="$(jq -r .version packages/textlint-plugin-asciidoc/package.json)"
test "$PACKAGE_TAG" = "textlint-plugin-asciidoc/v$version"
git rev-parse "refs/tags/$PACKAGE_TAG^{commit}"
git rev-parse HEAD
git merge-base --is-ancestor "$PACKAGE_COMMIT" refs/remotes/origin/main
cargo make test-textlint-plugin-release-candidate
`,
          },
        ],
      },
      publish: {
        needs: "candidate",
        environment: "npm-publish",
        permissions: { contents: "read", "id-token": "write" },
        steps: [
          { uses: pin },
          {
            run: `
node tools/npm-publication.mjs
npm publish "$tarball"
test "$predicate" = "https://slsa.dev/provenance/v1"
nix develop .#ci-browser -c node tools/textlint-plugin-npm-smoke.mjs
`,
          },
        ],
      },
    },
  };
}

function wasmPublicationWorkflow() {
  return {
    on: { push: { tags: ["wasm/v[0-9]+.[0-9]+.[0-9]+"] } },
    permissions: { contents: "read" },
    concurrency: {
      group: "wasm-publish-${{ github.ref_name }}",
      "cancel-in-progress": false,
    },
    jobs: {
      candidate: {
        steps: [
          {
            uses: pin,
            with: { "fetch-depth": 0, "fetch-tags": true, "persist-credentials": false },
          },
          {
            run: `
version="$(jq -r .version packages/wasm/package.json)"
test "$PACKAGE_TAG" = "wasm/v$version"
git rev-parse "refs/tags/$PACKAGE_TAG^{commit}"
git rev-parse HEAD
git merge-base --is-ancestor "$PACKAGE_COMMIT" refs/remotes/origin/main
cargo make test-wasm-release-candidate
`,
          },
        ],
      },
      publish: {
        needs: "candidate",
        environment: "npm-publish",
        permissions: { contents: "read", "id-token": "write" },
        steps: [
          { uses: pin },
          {
            run: `
node tools/npm-publication.mjs
npm publish "$tarball"
test "$predicate" = "https://slsa.dev/provenance/v1"
nix develop .#ci-browser -c node tools/wasm-npm-smoke.mjs
`,
          },
        ],
      },
    },
  };
}

function cachixPublicationWorkflow() {
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
    concurrency: { group: `cachix-publish-${publicationTag}`, "cancel-in-progress": false },
    jobs: {
      publish: {
        environment: "cachix-publish",
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
node tools/cachix-smoke.mjs "$package/bin/adocweave"
`,
          },
        ],
      },
    },
  };
}

function vscodePublicationWorkflow() {
  const candidate = {
    outputs: { version: "${{ steps.contract.outputs.version }}", vsix: "${{ steps.contract.outputs.vsix }}" },
    steps: [
      {
        uses: pin,
        with: { "fetch-depth": 0, "fetch-tags": true, "persist-credentials": false },
      },
      {
        run: `
version="$(jq -r .version editors/vscode/package.json)"
test "$PACKAGE_TAG" = "vscode/v$version"
tag_commit="$(git rev-parse "refs/tags/$PACKAGE_TAG^{commit}")"
test "$tag_commit" = "$PACKAGE_COMMIT"
git rev-parse HEAD
git merge-base --is-ancestor "$PACKAGE_COMMIT" refs/remotes/origin/main
cargo make test-vscode-release-candidate
`,
      },
      { uses: "actions/upload-artifact@0000000000000000000000000000000000000000", with: { name: "vscode-extension-candidate" } },
    ],
  };
  const registryJob = (environment) => ({
    needs: "candidate",
    environment,
    steps: [
      { uses: pin },
      { uses: "actions/download-artifact@0000000000000000000000000000000000000000", with: { name: "vscode-extension-candidate" } },
      { id: "existing", run: "node registry-check --name adocweave --version \"$VERSION\" --output result.vsix" },
      { if: "steps.existing.outputs.state == 'missing'", run: "publish" },
      { run: "node registry-check --name adocweave --version \"$VERSION\" --output result.vsix\ncargo make test-vscode-published-extension" },
    ],
  });
  const marketplace = registryJob("marketplace-publish");
  marketplace.permissions = { contents: "read", "id-token": "write" };
  marketplace.steps[3].run = "npx vsce publish --packagePath extension.vsix --oidc";
  const openVsx = registryJob("open-vsx-publish");
  openVsx.steps[3].env = { OVSX_PAT: "${{ secrets.OPEN_VSX_TOKEN }}" };
  openVsx.steps[3].run = "npx ovsx publish extension.vsix";
  return {
    on: { push: { tags: ["vscode/v[0-9]+.[0-9]+.[0-9]+"] } },
    permissions: { contents: "read" },
    concurrency: { group: "vscode-publish-${{ github.ref_name }}", "cancel-in-progress": false },
    jobs: { candidate, marketplace, "open-vsx": openVsx },
  };
}

function workflows() {
  return {
    "release.yml": releaseWorkflow(),
    "native-release-checks.yml": nativeReleaseChecksWorkflow(),
    "ci.yml": ciWorkflow(),
    "native-artifact-smoke.yml": {
      on: { workflow_call: {} },
      permissions: { contents: "read" },
      jobs: { smoke: { steps: [] } },
    },
    "cachix-publish.yml": cachixPublicationWorkflow(),
    "textlint-plugin-publish.yml": textlintPluginPublicationWorkflow(),
    "wasm-publish.yml": wasmPublicationWorkflow(),
    "vscode-publish.yml": vscodePublicationWorkflow(),
  };
}

function releaseFlowWorkflows(release) {
  return {
    "release.yml": release,
    "native-release-checks.yml": nativeReleaseChecksWorkflow(),
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

  fixtures["textlint-plugin-publish.yml"].jobs.publish.permissions["id-token"] = "read";
  assert.throws(() => validatePermissions(fixtures), /textlint-plugin-publish\.yml publish permissions/);
  fixtures["textlint-plugin-publish.yml"].jobs.publish.permissions["id-token"] = "write";

  fixtures["vscode-publish.yml"].jobs["open-vsx"].permissions = { contents: "write" };
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

test("native release checksはstable tagがmainに含まれることをhost前に一度だけ検査する", () => {
  const fixtures = releaseFlowWorkflows(releaseWorkflow());
  validateReleaseFlow(fixtures, 'pr-run-mode = "plan"');

  fixtures["native-release-checks.yml"].jobs.verify.steps[0].with["fetch-depth"] = 1;
  assert.throws(
    () => validateReleaseFlow(fixtures, 'pr-run-mode = "plan"'),
    /fetch complete history/u,
  );

  const late = releaseFlowWorkflows(releaseWorkflow());
  late["native-release-checks.yml"].jobs.verify.steps.reverse();
  assert.throws(
    () => validateReleaseFlow(late, 'pr-run-mode = "plan"'),
    /main ancestry once after tag and version checks/u,
  );

  const bypassed = releaseFlowWorkflows(releaseWorkflow());
  bypassed["release.yml"].jobs.host.needs = bypassed["release.yml"].jobs.host.needs.filter(
    (job) => job !== "custom-native-release-checks",
  );
  assert.throws(
    () => validateReleaseFlow(bypassed, 'pr-run-mode = "plan"'),
    /host must wait for.*native checks/u,
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

test("hostはcargo-distの最終manifestを含む成果物だけを公開する", () => {
  const release = releaseWorkflow();
  release.jobs.host.steps[1].run = release.jobs.host.steps[1].run.replace(
    "rm -f artifacts/*-dist-manifest.json",
    "rm -f artifacts/dist-manifest.json",
  );
  assert.throws(
    () => validateReleaseFlow(releaseFlowWorkflows(release), 'pr-run-mode = "plan"'),
    /preserve and publish the cargo-dist manifest/u,
  );
});

test("native Release成功後はCachix公開workflowだけを呼び出す", () => {
  const release = releaseWorkflow();
  validateReleaseFlow(releaseFlowWorkflows(release), 'pr-run-mode = "plan"');

  release.jobs["publish-cachix"].uses = "./.github/workflows/wasm-publish.yml";
  assert.throws(
    () => validateReleaseFlow(releaseFlowWorkflows(release), 'pr-run-mode = "plan"'),
    /publish-cachix must call cachix-publish\.yml/u,
  );

  const announced = releaseWorkflow();
  announced.jobs.announce = { needs: ["host", "publish-cachix"], steps: [] };
  assert.throws(
    () => validateReleaseFlow(releaseFlowWorkflows(announced), 'pr-run-mode = "plan"'),
    /must not keep an empty announce job/u,
  );

  const npm = releaseWorkflow();
  npm.jobs["publish-wasm"] = releasePublicationJob("wasm-publish.yml", true);
  assert.throws(
    () => validateReleaseFlow(releaseFlowWorkflows(npm), 'pr-run-mode = "plan"'),
    /must call only native checks, native smoke, and Cachix publication/u,
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
  fixtures["cachix-publish.yml"].on.workflow_dispatch = {};
  assert.throws(
    () => validateExternalPublicationIsolation(fixtures),
    /cachix-publish.*callable only/u,
  );

  const called = workflows();
  called["cachix-publish.yml"].on.workflow_call.inputs.commit.required = false;
  assert.throws(
    () => validateExternalPublicationIsolation(called),
    /cachix-publish.*stable tag and complete commit/u,
  );

  const duplicateValidation = workflows();
  duplicateValidation["cachix-publish.yml"].jobs.validate = { steps: [] };
  assert.throws(
    () => validateExternalPublicationIsolation(duplicateValidation),
    /cachix-publish.*isolated publication jobs/u,
  );

  const duplicateAncestry = workflows();
  duplicateAncestry["cachix-publish.yml"].jobs.publish.steps.push({
    run: "git merge-base --is-ancestor HEAD refs/remotes/origin/main",
  });
  assert.throws(
    () => validateExternalPublicationIsolation(duplicateAncestry),
    /cachix-publish.*leave main ancestry verification.*native release checks/u,
  );

  const extraSender = workflows();
  extraSender["cachix-publish.yml"].jobs.publish.steps.push({
    run: 'gh api "repos/$GITHUB_REPOSITORY/dispatches"',
  });
  assert.throws(
    () => validateExternalPublicationIsolation(extraSender),
    /must not use dispatch events/u,
  );
});

test("Open VSXのtokenを専用workflowの公開job以外へ渡さない", () => {
  const fixtures = workflows();
  validateVscodePublication(fixtures);

  fixtures["cachix-publish.yml"].jobs.verify.steps.push({
    env: { TOKEN: "${{ secrets.OPEN_VSX_TOKEN }}" },
    run: "true",
  });
  assert.throws(
    () => validateVscodePublication(fixtures),
    /OPEN_VSX_TOKEN.*only by vscode-publish\.yml open-vsx/u,
  );
});

test("VS Code拡張は一つの候補を二つのregistryへ独立して公開する", () => {
  const fixtures = workflows();
  validateVscodePublication(fixtures);

  const marketplaceStep = fixtures["vscode-publish.yml"].jobs.marketplace.steps.find((step) =>
    String(step.run ?? "").includes("vsce publish")
  );
  marketplaceStep.run =
    "npx vsce publish --packagePath extension.vsix --azure-credential";
  assert.throws(
    () => validateVscodePublication(fixtures),
    /trusted publishing with OIDC/u,
  );

  const azure = workflows();
  azure["vscode-publish.yml"].jobs.marketplace.steps.push({
    uses: "Azure/login@0000000000000000000000000000000000000000",
  });
  assert.throws(
    () => validateVscodePublication(azure),
    /trusted publishing with OIDC/u,
  );

  const shared = workflows();
  shared["vscode-publish.yml"].jobs["open-vsx"].environment = "marketplace-publish";
  assert.throws(
    () => validateVscodePublication(shared),
    /separate GitHub environments/u,
  );
});

test("Cachix公開は二つのLinux closureを送り別のtokenなしrunnerで取得する", () => {
  const fixtures = workflows();
  validateCachixPublication(fixtures);

  fixtures["cachix-publish.yml"].jobs.verify.steps[2].run =
    fixtures["cachix-publish.yml"].jobs.verify.steps[2].run.replace(
      "--option max-jobs 0",
      "--option max-jobs 1",
    );
  assert.throws(
    () => validateCachixPublication(fixtures),
    /acquire and smoke.*max-jobs/u,
  );
});

test("Cachixの書込みtokenを公開job以外へ渡さない", () => {
  const fixtures = workflows();
  fixtures["cachix-publish.yml"].jobs.verify.steps[1].with.authToken =
    "${{ secrets.CACHIX_AUTH_TOKEN }}";
  assert.throws(
    () => validateCachixPublication(fixtures),
    /without a write token/u,
  );

  const separateFixtures = workflows();
  separateFixtures["vscode-publish.yml"].jobs["open-vsx"].steps.push({
    env: { TOKEN: "${{ secrets.CACHIX_AUTH_TOKEN }}" },
    run: "true",
  });
  assert.throws(
    () => validateCachixPublication(separateFixtures),
    /used only by cachix-publish/u,
  );
});

test("textlint packageは専用tagから構築してnpmへ直接公開する", () => {
  const fixtures = workflows();
  validateTextlintPluginPublication(fixtures, textlintNpmSmoke);

  fixtures["textlint-plugin-publish.yml"].on.push.tags = ["v[0-9]+.[0-9]+.[0-9]+"];
  assert.throws(
    () => validateTextlintPluginPublication(fixtures, textlintNpmSmoke),
    /stable textlint-plugin-asciidoc\/vX\.Y\.Z tags/u,
  );

  const releaseDependent = workflows();
  releaseDependent["textlint-plugin-publish.yml"].jobs.publish.steps.at(-1).run +=
    "\ngh release download";
  assert.throws(
    () => validateTextlintPluginPublication(releaseDependent, textlintNpmSmoke),
    /must not depend on a native Release/u,
  );

  assert.throws(
    () => validateTextlintPluginPublication(
      workflows(),
      "runTextlintPluginConsumerE2E(spec); runTextlintPluginNpxSmoke(spec);",
    ),
    /verify signatures, provenance, and fixed consumers/u,
  );
});

test("WebAssembly packageは専用tagから構築してnpmへ直接公開する", () => {
  const fixtures = workflows();
  validateWasmPublication(fixtures, wasmNpmSmoke);

  fixtures["wasm-publish.yml"].on.push.tags = ["v[0-9]+.[0-9]+.[0-9]+"];
  assert.throws(() => validateWasmPublication(fixtures, wasmNpmSmoke), /stable wasm\/vX\.Y\.Z tags/);

  const releaseDependent = workflows();
  releaseDependent["wasm-publish.yml"].jobs.publish.steps.at(-1).run +=
    "\ngh release download";
  assert.throws(
    () => validateWasmPublication(releaseDependent, wasmNpmSmoke),
    /must not depend on a native Release/,
  );

  assert.throws(
    () => validateWasmPublication(workflows(), "runWasmPackageBrowserSmoke(packageRoot)"),
    /verify signatures, provenance, and the browser package/,
  );
});

test("release guideはnative版の--checkと--versionだけを使う", () => {
  validateNativeVersionCommands(`
node tools/native-release-version.mjs --version X.Y.Z
node tools/native-release-version.mjs --check
`);
  assert.throws(
    () => validateNativeVersionCommands(
      "node tools/native-release-version.mjs --product cli --version X.Y.Z",
    ),
    /usage:/,
  );
});
