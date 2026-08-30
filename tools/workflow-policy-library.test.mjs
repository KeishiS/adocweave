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
      "build-local-artifacts": {
        needs: ["plan", "custom-native-release-checks"],
        steps: [
          {
            if: "runner.os == 'macOS'",
            shell: "bash",
            run: 'echo "MACOSX_DEPLOYMENT_TARGET=14.0" >> "$GITHUB_ENV"',
          },
          { run: "node tools/generate-third-party-notices.mjs THIRD_PARTY_NOTICES.adoc" },
          { run: "dist build --artifacts=local" },
        ],
      },
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
      ...cachixReleaseJobs(),
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
      "portable-project": {
        strategy: { matrix: { os: ["macos-15", "windows-2025"] } },
        steps: [{ run: "cargo test --locked -p adocweave-project --all-features" }],
      },
      verify: {
        if: "${{ always() }}",
        needs: ["source", "portable-project"],
        steps: [{
          env: {
            SOURCE_RESULT: "${{ needs.source.result }}",
            PORTABLE_PROJECT_RESULT: "${{ needs.portable-project.result }}",
          },
          run: 'test "$SOURCE_RESULT" = success && test "$PORTABLE_PROJECT_RESULT" = success',
        }],
      },
      security: { if: main, needs: ["verify"], steps: [] },
      "main-integrations": { if: main, needs: ["verify", "security"], steps: [] },
      "fuzz-smoke": { if: main, needs: ["verify", "security"], steps: [] },
      "nix-package-check": { if: main, needs: ["verify", "security"], steps: [] },
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
            id: "node-version",
            run: 'echo "value=$(jq -er .nodeVersion toolchains.json)" >> "$GITHUB_OUTPUT"',
          },
          { uses: "DeterminateSystems/determinate-nix-action@0000000000000000000000000000000000000000" },
          {
            uses: "actions/setup-node@0000000000000000000000000000000000000000",
            with: { "node-version": "${{ steps.node-version.outputs.value }}" },
          },
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
            id: "node-version",
            run: 'echo "value=$(jq -er .nodeVersion toolchains.json)" >> "$GITHUB_OUTPUT"',
          },
          { uses: "DeterminateSystems/determinate-nix-action@0000000000000000000000000000000000000000" },
          {
            uses: "actions/setup-node@0000000000000000000000000000000000000000",
            with: { "node-version": "${{ steps.node-version.outputs.value }}" },
          },
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

function cachixReleaseJobs() {
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
    "publish-cachix": {
      needs: ["plan", "host"],
      if: "${{ always() && needs.host.result == 'success' }}",
      environment: "cachix-publish",
      concurrency: {
        group: "cachix-publish-${{ needs.plan.outputs.tag }}-${{ matrix.nixSystem }}",
        "cancel-in-progress": false,
      },
      permissions: { contents: "read" },
      strategy: matrix,
      "runs-on": "${{ matrix.runner }}",
      steps: [
        {
          uses: pin,
          with: { ref: "${{ github.sha }}" },
        },
        {
          run: stableReleaseVerification,
        },
        {
          env: { CACHIX_AUTH_TOKEN: "${{ secrets.CACHIX_AUTH_TOKEN }}" },
          run: `if [[ -z "$CACHIX_AUTH_TOKEN" ]]; then
  echo "::error::CACHIX_AUTH_TOKEN is unavailable in the cachix-publish environment."
  exit 1
fi`,
        },
        { uses: "DeterminateSystems/determinate-nix-action@0000000000000000000000000000000000000000" },
        {
          uses: "cachix/cachix-action@0000000000000000000000000000000000000000",
          with: {
            name: "keishis",
            authToken: "${{ secrets.CACHIX_AUTH_TOKEN }}",
            skipPush: true,
          },
        },
        {
          env: { NIX_SYSTEM: "${{ matrix.nixSystem }}" },
          run: 'nix build ".#checks.${NIX_SYSTEM}.default"\ncachix push keishis "$package"',
        },
      ],
    },
    "verify-cachix": {
      needs: "publish-cachix",
      permissions: { contents: "read" },
      strategy: structuredClone(matrix),
      "runs-on": "${{ matrix.runner }}",
      steps: [
        { uses: pin, with: { ref: "${{ github.sha }}" } },
        {
          env: {
            CACHIX_PUBLIC_KEY:
              "keishis.cachix.org-1:j3UwGrrgTifYMa9Uo6fyDU8GEJBcorOzrHdkXBXruK4=",
            NIX_SYSTEM: "${{ matrix.nixSystem }}",
          },
          run: `
expected_package="$(nix eval --raw ".#packages.\${NIX_SYSTEM}.default.outPath")"
store_hash="$(basename "$expected_package" | cut -d- -f1)"
[[ "$store_hash" =~ ^[0-9abcdfghijklmnpqrsvwxyz]{32}$ ]]
curl --fail --silent --show-error --retry 5 \
  "https://keishis.cachix.org/\${store_hash}.narinfo" > "$narinfo"
grep -Fx "StorePath: $expected_package" "$narinfo"
nix build ".#packages.\${NIX_SYSTEM}.default" \\
  --builders '' \\
  --no-fallback \\
  --max-jobs 0 \\
  --extra-trusted-public-keys "$CACHIX_PUBLIC_KEY" \\
  --substituters "https://keishis.cachix.org https://cache.nixos.org"
test "$package" = "$expected_package"
node tools/cachix-smoke.mjs "$package/bin/adocweave"
`,
        },
      ],
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
  marketplace.env = {
    AZURE_CLIENT_ID: "${{ vars.AZURE_CLIENT_ID }}",
    AZURE_TENANT_ID: "${{ vars.AZURE_TENANT_ID }}",
  };
  marketplace.steps.splice(1, 0, {
    uses: "azure/login@0000000000000000000000000000000000000000",
    with: {
      "client-id": "${{ vars.AZURE_CLIENT_ID }}",
      "tenant-id": "${{ vars.AZURE_TENANT_ID }}",
      "allow-no-subscriptions": true,
    },
  });
  marketplace.steps[4].run =
    "npx vsce publish --packagePath extension.vsix --azure-credential";
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

test("native配布物の構築前に第三者依存の通知を生成する", () => {
  const valid = releaseFlowWorkflows(releaseWorkflow());
  validateReleaseFlow(valid, 'pr-run-mode = "plan"');

  const conditional = releaseFlowWorkflows(releaseWorkflow());
  const conditionalNotice = conditional["release.yml"].jobs["build-local-artifacts"].steps
    .find((step) => String(step.run).includes("generate-third-party-notices"));
  conditionalNotice.if = "runner.os == 'Linux'";
  assert.throws(
    () => validateReleaseFlow(conditional, 'pr-run-mode = "plan"'),
    /generate third-party notices unconditionally/u,
  );

  const late = releaseFlowWorkflows(releaseWorkflow());
  const lateSteps = late["release.yml"].jobs["build-local-artifacts"].steps;
  const lateNoticeIndex = lateSteps.findIndex((step) =>
    String(step.run).includes("generate-third-party-notices")
  );
  const [lateNotice] = lateSteps.splice(lateNoticeIndex, 1);
  lateSteps.push(lateNotice);
  assert.throws(
    () => validateReleaseFlow(late, 'pr-run-mode = "plan"'),
    /generate third-party notices before cargo-dist/u,
  );
});

test("macOS配布物は14.0を対応下限として構築する", () => {
  const valid = releaseFlowWorkflows(releaseWorkflow());
  validateReleaseFlow(valid, 'pr-run-mode = "plan"');

  const wrongRunner = releaseFlowWorkflows(releaseWorkflow());
  wrongRunner["release.yml"].jobs["build-local-artifacts"].steps[0].if =
    "runner.os == 'Linux'";
  assert.throws(
    () => validateReleaseFlow(wrongRunner, 'pr-run-mode = "plan"'),
    /append the macOS 14\.0 deployment target only on macOS/u,
  );

  const missingRedirect = releaseFlowWorkflows(releaseWorkflow());
  missingRedirect["release.yml"].jobs["build-local-artifacts"].steps[0].run =
    'echo "MACOSX_DEPLOYMENT_TARGET=14.0" "$GITHUB_ENV"';
  assert.throws(
    () => validateReleaseFlow(missingRedirect, 'pr-run-mode = "plan"'),
    /append the macOS 14\.0 deployment target only on macOS/u,
  );

  const late = releaseFlowWorkflows(releaseWorkflow());
  const lateSteps = late["release.yml"].jobs["build-local-artifacts"].steps;
  const [macosTarget] = lateSteps.splice(0, 1);
  lateSteps.push(macosTarget);
  assert.throws(
    () => validateReleaseFlow(late, 'pr-run-mode = "plan"'),
    /set the macOS deployment target before cargo-dist/u,
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

test("native Release成功後はCachix公開jobを保護されたenvironmentで直接実行する", () => {
  const release = releaseWorkflow();
  validateReleaseFlow(releaseFlowWorkflows(release), 'pr-run-mode = "plan"');

  release.jobs["publish-cachix"].environment = "other-environment";
  assert.throws(
    () => validateReleaseFlow(releaseFlowWorkflows(release), 'pr-run-mode = "plan"'),
    /publish-cachix must select its protected environment directly/u,
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
    /must call only native checks and native smoke workflows/u,
  );
});

test("CIはPRのsource gateとmain専用gateを分離する", () => {
  const ci = ciWorkflow();
  validateCiGates({ "ci.yml": ci });
  delete ci.jobs["main-integrations"].if;
  assert.throws(() => validateCiGates({ "ci.yml": ci }), /main-integrations.*main-only/);

  const missingPortable = ciWorkflow();
  delete missingPortable.jobs["portable-project"];
  assert.throws(
    () => validateCiGates({ "ci.yml": missingPortable }),
    /portable project gate/u,
  );

  const incompleteAggregation = ciWorkflow();
  delete incompleteAggregation.jobs.verify.steps[0].env.PORTABLE_PROJECT_RESULT;
  assert.throws(
    () => validateCiGates({ "ci.yml": incompleteAggregation }),
    /verify gate must aggregate/u,
  );

  const bypassedAggregation = ciWorkflow();
  bypassedAggregation.jobs.security.needs = ["source"];
  assert.throws(
    () => validateCiGates({ "ci.yml": bypassedAggregation }),
    /security.*verify-gated/u,
  );
});

test("Cachix公開は再利用workflowやdispatchを経由しない", () => {
  const fixtures = workflows();
  validateExternalPublicationIsolation(fixtures);

  fixtures["cachix-publish.yml"] = {
    on: { workflow_call: {} },
    permissions: { contents: "read" },
    jobs: {},
  };
  assert.throws(
    () => validateExternalPublicationIsolation(fixtures),
    /must not cross a reusable workflow secrets boundary/u,
  );

  const extraSender = workflows();
  extraSender["release.yml"].jobs["publish-cachix"].steps.push({
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

  fixtures["release.yml"].jobs["verify-cachix"].steps.push({
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
  marketplaceStep.run = "npx vsce publish --packagePath extension.vsix --oidc";
  assert.throws(
    () => validateVscodePublication(fixtures),
    /Microsoft Entra ID credential/u,
  );

  const azure = workflows();
  const azureLogin = azure["vscode-publish.yml"].jobs.marketplace.steps.find((step) =>
    String(step.uses ?? "").startsWith("azure/login@")
  );
  azureLogin.with["client-id"] = "${{ secrets.AZURE_CLIENT_ID }}";
  assert.throws(
    () => validateVscodePublication(azure),
    /federated Azure identity from environment variables/u,
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

  const verify = fixtures["release.yml"].jobs["verify-cachix"];
  const smoke = verify.steps.find((step) => String(step.run ?? "").includes("max-jobs"));
  smoke.run = smoke.run.replace(
    "--max-jobs 0",
    "--max-jobs 1",
  );
  assert.throws(
    () => validateCachixPublication(fixtures),
    /acquire and smoke.*max-jobs/u,
  );

  const wrongRunner = workflows();
  wrongRunner["release.yml"].jobs["publish-cachix"]["runs-on"] = "ubuntu-24.04";
  assert.throws(
    () => validateCachixPublication(wrongRunner),
    /fixed x86_64-linux and aarch64-linux targets/u,
  );

  const wrongBuildSystem = workflows();
  const build = wrongBuildSystem["release.yml"].jobs["publish-cachix"].steps.find((step) =>
    String(step.run ?? "").includes("cachix push")
  );
  build.env.NIX_SYSTEM = "x86_64-linux";
  assert.throws(
    () => validateCachixPublication(wrongBuildSystem),
    /build the Nix system selected by its matrix entry/u,
  );

  const wrongVerifySystem = workflows();
  const verifySmoke = wrongVerifySystem["release.yml"].jobs["verify-cachix"].steps.find((step) =>
    String(step.run ?? "").includes("cachix-smoke")
  );
  verifySmoke.env.NIX_SYSTEM = "x86_64-linux";
  assert.throws(
    () => validateCachixPublication(wrongVerifySystem),
    /acquire the Nix system selected by its matrix entry/u,
  );

  const wrongPublicKey = workflows();
  const untrustedSmoke = wrongPublicKey["release.yml"].jobs["verify-cachix"].steps.find((step) =>
    String(step.run ?? "").includes("cachix-smoke")
  );
  untrustedSmoke.env.CACHIX_PUBLIC_KEY = "keishis.cachix.org-1:wrong";
  assert.throws(
    () => validateCachixPublication(wrongPublicKey),
    /pin the trusted Cachix public key/u,
  );

  const missingUpstream = workflows();
  const incompleteSmoke = missingUpstream["release.yml"].jobs["verify-cachix"].steps.find((step) =>
    String(step.run ?? "").includes("cachix-smoke")
  );
  incompleteSmoke.run = incompleteSmoke.run.replace(
    "https://keishis.cachix.org https://cache.nixos.org",
    "https://keishis.cachix.org",
  );
  assert.throws(
    () => validateCachixPublication(missingUpstream),
    /acquire and smoke.*cache\.nixos\.org/u,
  );

  const duplicateAncestry = workflows();
  duplicateAncestry["release.yml"].jobs["publish-cachix"].steps.push({
    run: "git merge-base --is-ancestor HEAD refs/remotes/origin/main",
  });
  assert.throws(
    () => validateCachixPublication(duplicateAncestry),
    /leave main ancestry verification in native checks/u,
  );
});

test("Cachixの書込みtokenを公開job以外へ渡さない", () => {
  const fixtures = workflows();
  fixtures["release.yml"].jobs["verify-cachix"].steps.push({
    uses: "cachix/cachix-action@0000000000000000000000000000000000000000",
    with: { authToken: "${{ secrets.CACHIX_AUTH_TOKEN }}", name: "keishis" },
  });
  assert.throws(
    () => validateCachixPublication(fixtures),
    /only the pinned public cache identity/u,
  );

  const jobWide = workflows();
  jobWide["release.yml"].jobs["publish-cachix"].env = {
    CACHIX_AUTH_TOKEN: "${{ secrets.CACHIX_AUTH_TOKEN }}",
  };
  assert.throws(
    () => validateCachixPublication(jobWide),
    /use only the dedicated Cachix token/u,
  );

  const separateFixtures = workflows();
  separateFixtures["vscode-publish.yml"].jobs["open-vsx"].steps.push({
    env: { TOKEN: "${{ secrets.CACHIX_AUTH_TOKEN }}" },
    run: "true",
  });
  assert.throws(
    () => validateCachixPublication(separateFixtures),
    /used only by release\.yml publish-cachix/u,
  );
});

test("Cachix認証情報がなければpackage構築前に停止する", () => {
  const fixtures = workflows();
  const publish = fixtures["release.yml"].jobs["publish-cachix"];
  publish.steps = publish.steps.filter((step) =>
    !String(step.run ?? "").includes("CACHIX_AUTH_TOKEN is unavailable")
  );
  assert.throws(
    () => validateCachixPublication(fixtures),
    /fail before building when authentication is unavailable/u,
  );

  const late = workflows();
  const lateSteps = late["release.yml"].jobs["publish-cachix"].steps;
  const authenticationIndex = lateSteps.findIndex((step) =>
    String(step.run ?? "").includes("CACHIX_AUTH_TOKEN is unavailable")
  );
  lateSteps.push(...lateSteps.splice(authenticationIndex, 1));
  assert.throws(
    () => validateCachixPublication(late),
    /fail before building when authentication is unavailable/u,
  );

  const ignored = workflows();
  const authentication = ignored["release.yml"].jobs["publish-cachix"].steps.find((step) =>
    String(step.run ?? "").includes("CACHIX_AUTH_TOKEN is unavailable")
  );
  authentication["continue-on-error"] = true;
  assert.throws(
    () => validateCachixPublication(ignored),
    /fail before building when authentication is unavailable/u,
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

  const overriddenNode = workflows();
  const textlintSteps = overriddenNode["textlint-plugin-publish.yml"].jobs.publish.steps;
  const nixIndex = textlintSteps.findIndex((step) =>
    String(step.uses ?? "").startsWith("DeterminateSystems/determinate-nix-action@")
  );
  const [nix] = textlintSteps.splice(nixIndex, 1);
  textlintSteps.push(nix);
  assert.throws(
    () => validateTextlintPluginPublication(overriddenNode, textlintNpmSmoke),
    /set up pinned Node\.js after Nix/u,
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

  const overriddenNode = workflows();
  const wasmSteps = overriddenNode["wasm-publish.yml"].jobs.publish.steps;
  const nixIndex = wasmSteps.findIndex((step) =>
    String(step.uses ?? "").startsWith("DeterminateSystems/determinate-nix-action@")
  );
  const [nix] = wasmSteps.splice(nixIndex, 1);
  wasmSteps.push(nix);
  assert.throws(
    () => validateWasmPublication(overriddenNode, wasmNpmSmoke),
    /set up pinned Node\.js after Nix/u,
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
