import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { parseReleaseVersionArguments } from "./sync-release-version.mjs";

// actionlint checks workflow syntax. This policy checks repository-specific
// trust boundaries that generated cargo-dist YAML cannot express by itself.

const ROOT = new URL("../", import.meta.url);
const read = (path) => readFileSync(new URL(path, ROOT), "utf8");

const READ_ONLY = { contents: "read" };
const CI_READ_ONLY = { actions: "read", contents: "read" };
const RELEASE_HOST = {
  attestations: "write",
  contents: "write",
  "id-token": "write",
};
const PUBLICATION_TAG_EXPRESSION = "${{ inputs.tag }}";
const PUBLICATION_COMMIT_EXPRESSION = "${{ inputs.commit }}";
const OIDC_PUBLICATION = { contents: "read", "id-token": "write" };
const MARKETPLACE_OIDC_PUBLICATION = { contents: "read", "id-token": "write" };
const EXTERNAL_PUBLICATION_WORKFLOWS = new Set([
  "binary-cache-publish.yml",
]);
const OIDC_PUBLICATION_WORKFLOWS = new Set([
  "textlint-plugin-publish.yml",
  "wasm-publish.yml",
]);
const RELEASE_PUBLICATION_JOBS = new Map([
  ["publish-binary-cache", "binary-cache-publish.yml"],
]);
const BINARY_CACHE_TARGETS = [
  { runner: "ubuntu-24.04", nixSystem: "x86_64-linux" },
  { runner: "ubuntu-24.04-arm", nixSystem: "aarch64-linux" },
];

function fail(message) {
  throw new Error(message);
}

function canonical(value) {
  return JSON.stringify(
    Object.fromEntries(
      Object.entries(value ?? {}).sort(([left], [right]) => left.localeCompare(right)),
    ),
  );
}

function expectPermissions(actual, expected, location) {
  if (canonical(actual) !== canonical(expected)) {
    fail(`${location} must declare exactly ${canonical(expected)}`);
  }
}

function parseWorkflow(name, source) {
  const directory = mkdtempSync(join(tmpdir(), "adocweave-workflow-policy-"));
  const path = join(directory, "workflow.yml");
  writeFileSync(path, source);
  const parsed = spawnSync("yq", ["-o=json", ".", path], { encoding: "utf8" });
  rmSync(directory, { force: true, recursive: true });
  if (parsed.status !== 0) {
    fail(`cannot parse workflow ${name}: ${parsed.stderr.trim() || parsed.error?.message}`);
  }
  return JSON.parse(parsed.stdout);
}

function* workflowUses(document) {
  for (const [jobName, job] of Object.entries(document.jobs ?? {})) {
    if (typeof job.uses === "string") yield { location: `job ${jobName}`, value: job.uses };
    for (const step of job.steps ?? []) {
      if (typeof step.uses === "string") yield { location: `job ${jobName}`, value: step.uses };
    }
  }
}

function needs(job) {
  if (Array.isArray(job?.needs)) return job.needs;
  return typeof job?.needs === "string" ? [job.needs] : [];
}

function condition(job) {
  return String(job?.if ?? "").replace(/^\$\{\{\s*|\s*\}\}$/g, "");
}

function jobRuns(job) {
  return (job?.steps ?? [])
    .map((step) => step.run)
    .filter((run) => typeof run === "string")
    .join("\n");
}

export function validatePinnedActions(workflows) {
  for (const [name, document] of Object.entries(workflows)) {
    for (const { location, value } of workflowUses(document)) {
      if (value.startsWith("./")) continue;
      if (!/@[0-9a-f]{40}$/.test(value)) {
        fail(`${name} ${location} uses an action that is not pinned to a full commit SHA: ${value}`);
      }
    }
  }
}

export function validatePermissions(workflows) {
  for (const [name, workflow] of Object.entries(workflows)) {
    if (name === "release.yml") {
      expectPermissions(workflow.permissions, READ_ONLY, `${name} top-level permissions`);
    } else if (name === "ci.yml") {
      expectPermissions(workflow.permissions, CI_READ_ONLY, `${name} top-level permissions`);
    } else {
      expectPermissions(workflow.permissions, READ_ONLY, `${name} top-level permissions`);
    }

    for (const [jobName, job] of Object.entries(workflow.jobs ?? {})) {
      if (name === "release.yml" && jobName === "host") {
        expectPermissions(job.permissions, RELEASE_HOST, `${name} host permissions`);
        continue;
      }
      if (name === "release.yml" && RELEASE_PUBLICATION_JOBS.has(jobName)) {
        const calledWorkflow = RELEASE_PUBLICATION_JOBS.get(jobName);
        expectPermissions(
          job.permissions,
          OIDC_PUBLICATION_WORKFLOWS.has(calledWorkflow) ? OIDC_PUBLICATION : READ_ONLY,
          `${name} ${jobName} permissions`,
        );
        continue;
      }
      if (OIDC_PUBLICATION_WORKFLOWS.has(name) && jobName === "publish") {
        expectPermissions(
          job.permissions,
          OIDC_PUBLICATION,
          `${name} publish permissions`,
        );
        continue;
      }
      if (name === "vscode-publish.yml" && jobName === "marketplace") {
        expectPermissions(
          job.permissions,
          MARKETPLACE_OIDC_PUBLICATION,
          `${name} marketplace permissions`,
        );
        continue;
      }
      for (const [scope, level] of Object.entries(job.permissions ?? {})) {
        if (level !== "read" && level !== "none") {
          fail(`${name} job ${jobName} grants unexpected ${scope}: ${level}`);
        }
      }
    }
  }
}

export function validateReleaseFlow(workflows, distConfiguration) {
  const release = workflows["release.yml"];
  if (!release) fail("release.yml is required");
  const triggers = release.on ?? {};
  const triggerNames = Object.keys(triggers).sort();
  if (canonical(triggerNames) !== canonical(["pull_request", "push"])) {
    fail("release.yml must run only for pull requests and tag pushes");
  }
  const push = triggers.push ?? {};
  if (canonical(push.tags) !== canonical(["v[0-9]+.[0-9]+.[0-9]+"]) || push.branches !== undefined) {
    fail("release.yml push trigger must select stable vX.Y.Z tags only");
  }
  if (!/^pr-run-mode\s*=\s*"plan"$/mu.test(distConfiguration)) {
    fail("cargo-dist pull requests must use plan mode");
  }

  const jobs = release.jobs ?? {};
  const plan = jobs.plan;
  if (!plan || needs(plan).length !== 0 || plan.if !== undefined) {
    fail("release plan must be the unconditional root job");
  }
  if (plan.outputs?.publishing !== "${{ !github.event.pull_request }}") {
    fail("release plan must distinguish tag publication from pull-request planning");
  }

  const contractCalls = Object.entries(jobs).filter(([, job]) =>
    job.uses === "./.github/workflows/release-contract.yml"
  );
  if (contractCalls.length !== 1) {
    fail("release must call the common release contract exactly once");
  }
  const [contractJobName, contractCall] = contractCalls[0];
  if (needs(contractCall).length !== 0 || contractCall.if !== undefined ||
      contractCall.strategy !== undefined) {
    fail("common release contract must be one unconditional root job");
  }
  const contract = workflows["release-contract.yml"];
  const contractJobs = Object.entries(contract?.jobs ?? {});
  if (contract?.on?.workflow_call === undefined || contractJobs.length !== 1) {
    fail("release-contract.yml must expose one common workflow_call job");
  }
  const [, contractVerification] = contractJobs[0];
  const contractSteps = contractVerification.steps ?? [];
  const contractCheckout = contractSteps.find((step) =>
    typeof step.uses === "string" && step.uses.startsWith("actions/checkout@")
  );
  if (contractCheckout?.with?.["fetch-depth"] !== 0 ||
      contractCheckout?.with?.["persist-credentials"] !== false) {
    fail("common release contract must fetch complete history without checkout credentials");
  }
  const releaseContractIndex = contractSteps.findIndex((step) =>
    String(step.run ?? "").includes("cargo make release-contract")
  );
  const ancestrySteps = contractSteps
    .map((step, index) => ({ index, step }))
    .filter(({ step }) => String(step.run ?? "").includes("git merge-base --is-ancestor"));
  if (releaseContractIndex < 0 || ancestrySteps.length !== 1 ||
      ancestrySteps[0].index <= releaseContractIndex) {
    fail("common release contract must verify main ancestry once after tag and version checks");
  }
  const ancestry = ancestrySteps[0].step;
  const ancestrySource = String(ancestry.run ?? "");
  for (const required of [
    'git rev-parse "refs/tags/$RELEASE_TAG^{commit}"',
    'test "$tag_commit" = "$RELEASE_COMMIT"',
    'git merge-base --is-ancestor "$tag_commit" refs/remotes/origin/main',
  ]) {
    if (!ancestrySource.includes(required)) {
      fail(`common release contract must bind the stable tag to origin/main: ${required}`);
    }
  }
  if (!condition(ancestry).includes("github.event_name == 'push'") ||
      ancestry.env?.RELEASE_COMMIT !== "${{ github.sha }}" ||
      ancestry.env?.RELEASE_TAG !== "${{ github.ref_name }}") {
    fail("common release contract main ancestry check must use the triggering tag commit");
  }

  const host = jobs.host;
  const required = [
    "plan",
    contractJobName,
    "build-local-artifacts",
    "build-global-artifacts",
    "custom-native-artifact-smoke",
  ];
  if (!host || !required.every((jobName) => needs(host).includes(jobName))) {
    fail("release host must wait for plan, common contract, local, global, and native smoke jobs");
  }
  if (host.environment !== "github-release") {
    fail("release host write access and OIDC must stay in the github-release environment");
  }
  const hostCondition = condition(host);
  for (const jobName of required) {
    if (!hostCondition.includes(`needs.${jobName}.result == 'success'`)) {
      fail(`release host must require ${jobName} success`);
    }
  }
  if (!hostCondition.includes("needs.plan.outputs.publishing == 'true'")) {
    fail("release host must run only for a publishing tag push");
  }
  if (/result\s*==\s*'skipped'/.test(hostCondition)) {
    fail("release host must not treat a skipped prerequisite as success");
  }

  const attestation = (host.steps ?? []).find(
    (step) => typeof step.uses === "string" &&
      step.uses.startsWith("actions/attest-build-provenance@"),
  );
  if (String(attestation?.with?.["subject-path"] ?? "").trim() !== "artifacts/*") {
    fail("release host must attest every published artifact");
  }

  for (const [jobName, workflowName] of RELEASE_PUBLICATION_JOBS) {
    const publication = jobs[jobName];
    if (!publication ||
        canonical(needs(publication).sort()) !== canonical(["host", "plan"])) {
      fail(`release ${jobName} must run directly after plan and host`);
    }
    if (publication.uses !== `./.github/workflows/${workflowName}`) {
      fail(`release ${jobName} must call ${workflowName} directly`);
    }
    if (!condition(publication).includes("needs.host.result == 'success'")) {
      fail(`release ${jobName} must require successful GitHub Release creation`);
    }
    if (canonical(publication.with) !== canonical({
      tag: "${{ needs.plan.outputs.tag }}",
      commit: "${{ github.sha }}",
    })) {
      fail(`release ${jobName} must pass only the planned tag and complete commit SHA`);
    }
    if (publication.secrets !== undefined) {
      fail(`release ${jobName} must leave publication secrets in the called environment`);
    }
  }

  const announce = jobs.announce;
  const publicationJobs = [...RELEASE_PUBLICATION_JOBS.keys()];
  if (!announce || !["host", "plan", ...publicationJobs].every((jobName) =>
    needs(announce).includes(jobName)
  )) {
    fail("release announce must wait for every external publication job");
  }
  const announceCondition = condition(announce);
  for (const jobName of ["host", ...publicationJobs]) {
    if (!announceCondition.includes(`needs.${jobName}.result == 'success'`)) {
      fail(`release announce must require ${jobName} success`);
    }
  }
  if (/result\s*==\s*'skipped'/.test(announceCondition)) {
    fail("release announce must not report success after a skipped publication");
  }
}

export function validateCiGates(workflows) {
  const ci = workflows["ci.yml"];
  if (!ci) fail("ci.yml is required");
  if (ci.on?.pull_request === undefined || canonical(ci.on?.push?.branches) !== canonical(["main"])) {
    fail("CI must run for pull requests and pushes to main");
  }
  const jobs = ci.jobs ?? {};
  const source = jobs.source;
  if (!source || source.if !== undefined || needs(source).length !== 0 ||
      !jobRuns(source).includes("cargo make verify")) {
    fail("CI source gate must run cargo make verify for every CI event");
  }

  const expectedMainJobs = ["security", "main-integrations", "fuzz-smoke", "nix-package-check"];
  for (const jobName of expectedMainJobs) {
    const job = jobs[jobName];
    const mainOnly = condition(job);
    if (!job || !mainOnly.includes("github.event_name == 'push'") ||
        !mainOnly.includes("github.ref == 'refs/heads/main'") || !needs(job).includes("source")) {
      fail(`CI ${jobName} must be a source-gated main-only job`);
    }
  }
  for (const jobName of ["main-integrations", "fuzz-smoke", "nix-package-check"]) {
    if (!needs(jobs[jobName]).includes("security")) {
      fail(`CI ${jobName} must wait for the security gate`);
    }
  }
}

export function validateExternalPublicationIsolation(workflows) {
  for (const name of EXTERNAL_PUBLICATION_WORKFLOWS) {
    const workflow = workflows[name];
    if (!workflow) fail(`${name} is required`);
    const triggerNames = Object.keys(workflow.on ?? {}).sort();
    if (canonical(triggerNames) !== canonical(["workflow_call"])) {
      fail(`${name} must be callable only from the Release workflow`);
    }
    const inputs = workflow.on?.workflow_call?.inputs ?? {};
    if (canonical(Object.keys(inputs).sort()) !== canonical(["commit", "tag"]) ||
        inputs.tag?.required !== true || inputs.tag?.type !== "string" ||
        inputs.commit?.required !== true || inputs.commit?.type !== "string") {
      fail(`${name} must require only the stable tag and complete commit SHA`);
    }
    const configuration = JSON.stringify(workflow);
    if (!configuration.includes(PUBLICATION_TAG_EXPRESSION) ||
        !configuration.includes(PUBLICATION_COMMIT_EXPRESSION)) {
      fail(`${name} must consume the Release workflow tag and commit`);
    }
    if (workflow.concurrency?.["cancel-in-progress"] !== false ||
        !String(workflow.concurrency?.group ?? "").includes(PUBLICATION_TAG_EXPRESSION)) {
      fail(`${name} must serialize idempotent publication attempts by release tag`);
    }
    const expectedJobs = name === "binary-cache-publish.yml"
      ? ["publish", "verify"]
      : ["publish"];
    if (canonical(Object.keys(workflow.jobs ?? {}).sort()) !== canonical(expectedJobs.sort())) {
      fail(`${name} must contain only its isolated publication jobs`);
    }
    const publication = workflow.jobs.publish;
    if (needs(publication).length !== 0 ||
        !JSON.stringify(publication).includes('ref":"${{ inputs.commit }}"')) {
      fail(`${name} publication must start from the Release workflow commit`);
    }
    const expectedEnvironment = name.replace(/\.yml$/u, "");
    if (publication.environment !== expectedEnvironment) {
      fail(`${name} publication credentials must stay in the ${expectedEnvironment} environment`);
    }
    const publicationSource = jobRuns(publication);
    if (/merge-base\s+--is-ancestor|refs\/remotes\/origin\/main/u.test(publicationSource)) {
      fail(`${name} must leave main ancestry verification in the common release contract`);
    }
    for (const required of [
      'releases/tags/$RELEASE_TAG',
      ".draft",
      ".prerelease",
      'git rev-parse "refs/tags/$RELEASE_TAG^{commit}"',
      "git rev-parse HEAD",
      "workspace_version",
      'test "$RELEASE_TAG" = "v$workspace_version"',
    ]) {
      if (!publicationSource.includes(required)) {
        fail(`${name} publication must verify the stable Release candidate: ${required}`);
      }
    }
  }

  if (/repository_dispatch|workflow_dispatch|repos\/\$GITHUB_REPOSITORY\/dispatches/u.test(
    JSON.stringify(workflows),
  )) {
    fail("external publication must not use dispatch events or custom event delivery");
  }

}

function cachixAction(job) {
  return (job?.steps ?? []).find((step) =>
    typeof step.uses === "string" && step.uses.startsWith("cachix/cachix-action@")
  );
}

function validateBinaryCacheMatrix(job, location) {
  if (job?.strategy?.["fail-fast"] !== false ||
      canonical(job?.strategy?.matrix?.include) !== canonical(BINARY_CACHE_TARGETS)) {
    fail(`${location} must process the fixed x86_64-linux and aarch64-linux targets`);
  }
}

export function validateBinaryCachePublication(workflows) {
  const workflow = workflows["binary-cache-publish.yml"];
  if (!workflow) fail("binary-cache-publish.yml is required");
  const publish = workflow.jobs?.publish;
  const verify = workflow.jobs?.verify;
  validateBinaryCacheMatrix(publish, "binary-cache-publish.yml publish");
  validateBinaryCacheMatrix(verify, "binary-cache-publish.yml verify");
  if (!needs(verify).includes("publish")) {
    fail("binary-cache-publish.yml verify must run after every published closure");
  }

  const publishSource = jobRuns(publish);
  for (const required of [
    'releases/tags/$RELEASE_TAG',
    ".draft",
    ".prerelease",
    'git rev-parse "refs/tags/$RELEASE_TAG^{commit}"',
    "git rev-parse HEAD",
    "workspace_version",
    'test "$RELEASE_TAG" = "v$workspace_version"',
    'cachix push keishis "$package"',
  ]) {
    if (!publishSource.includes(required)) {
      fail(`binary-cache-publish.yml publish must verify and send the stable release: ${required}`);
    }
  }
  const publishCachix = cachixAction(publish);
  if (publishCachix?.with?.name !== "keishis" || publishCachix?.with?.skipPush !== true ||
      publishCachix?.with?.authToken !== "${{ secrets.CACHIX_AUTH_TOKEN }}") {
    fail("binary-cache-publish.yml publish must use only the dedicated Cachix token");
  }

  const verifySource = jobRuns(verify);
  for (const required of [
    '.#packages.${NIX_SYSTEM}.default',
    "--option builders ''",
    "--option fallback false",
    "--option max-jobs 0",
    "--option substituters https://keishis.cachix.org",
    "node tools/binary-cache-smoke.mjs",
  ]) {
    if (!verifySource.includes(required)) {
      fail(`binary-cache-publish.yml verify must acquire and smoke the public closure: ${required}`);
    }
  }
  const verifyCachix = cachixAction(verify);
  if (verifyCachix?.with?.name !== "keishis" || verifyCachix?.with?.skipPush !== true ||
      verifyCachix?.with?.authToken !== undefined ||
      JSON.stringify(verify).includes("CACHIX_AUTH_TOKEN")) {
    fail("binary-cache-publish.yml verify must configure Cachix without a write token");
  }

  const tokenUsers = Object.entries(workflows)
    .filter(([, candidate]) => JSON.stringify(candidate).includes("CACHIX_AUTH_TOKEN"))
    .map(([name]) => name);
  if (canonical(tokenUsers) !== canonical(["binary-cache-publish.yml"])) {
    fail("CACHIX_AUTH_TOKEN must be used only by binary-cache-publish.yml");
  }
}

export function validateTextlintPluginPublication(workflows, npmSmokeSource) {
  const name = "textlint-plugin-publish.yml";
  const workflow = workflows[name];
  if (!workflow) fail(`${name} is required`);
  if (canonical(Object.keys(workflow.on ?? {})) !== canonical(["push"]) ||
      canonical(workflow.on?.push?.tags) !==
        canonical(["textlint-plugin-asciidoc/v[0-9]+.[0-9]+.[0-9]+"])) {
    fail(`${name} must run only for stable textlint-plugin-asciidoc/vX.Y.Z tags`);
  }
  if (workflow.concurrency?.["cancel-in-progress"] !== false ||
      !String(workflow.concurrency?.group ?? "").includes("${{ github.ref_name }}")) {
    fail(`${name} must serialize idempotent publication attempts by package tag`);
  }
  if (canonical(Object.keys(workflow.jobs ?? {}).sort()) !== canonical(["candidate", "publish"])) {
    fail(`${name} must contain only candidate and publish jobs`);
  }
  const candidate = workflow.jobs.candidate;
  const publish = workflow.jobs.publish;
  if (needs(candidate).length !== 0 || canonical(needs(publish)) !== canonical(["candidate"])) {
    fail(`${name} must verify one candidate before publication`);
  }
  if (publish.environment !== "npm-publish") {
    fail(`${name} must keep Trusted Publishing in the npm-publish environment`);
  }
  const checkout = (candidate.steps ?? []).find((step) =>
    typeof step.uses === "string" && step.uses.startsWith("actions/checkout@")
  );
  if (checkout?.with?.["fetch-depth"] !== 0 || checkout?.with?.["fetch-tags"] !== true ||
      checkout?.with?.["persist-credentials"] !== false) {
    fail(`${name} must fetch tag and main history without checkout credentials`);
  }
  const candidateSource = jobRuns(candidate);
  for (const required of [
    "packages/textlint-plugin-asciidoc/package.json",
    'test "$PACKAGE_TAG" = "textlint-plugin-asciidoc/v$version"',
    'git rev-parse "refs/tags/$PACKAGE_TAG^{commit}"',
    "git rev-parse HEAD",
    'git merge-base --is-ancestor "$PACKAGE_COMMIT" refs/remotes/origin/main',
    "cargo make test-textlint-plugin-release-candidate",
  ]) {
    if (!candidateSource.includes(required)) {
      fail(`${name} must bind the package tag, version, source, and candidate: ${required}`);
    }
  }
  const publishSource = jobRuns(publish);
  for (const required of [
    "tools/npm-publication.mjs",
    'npm publish "$tarball"',
    "https://slsa.dev/provenance/v1",
    "nix develop .#ci-browser -c node tools/textlint-plugin-npm-smoke.mjs",
  ]) {
    if (!publishSource.includes(required)) {
      fail(`${name} must publish and verify the candidate directly on npm: ${required}`);
    }
  }
  const configuration = JSON.stringify(workflow);
  if (/gh release|releases\/tags|workspace_version|adocweave-textlint\/v/iu.test(configuration)) {
    fail(`${name} must not depend on a native Release, legacy tag, or workspace version`);
  }
  for (const required of [
    '"audit"',
    '"signatures"',
    '"--include-attestations"',
    "runTextlintPluginConsumerE2E",
    "runTextlintPluginNpxSmoke",
  ]) {
    if (!npmSmokeSource?.includes(required)) {
      fail(`textlint npm smoke must verify signatures, provenance, and fixed consumers: ${required}`);
    }
  }
}

export function validateWasmPublication(workflows, npmSmokeSource) {
  const workflow = workflows["wasm-publish.yml"];
  if (!workflow) fail("wasm-publish.yml is required");
  if (canonical(Object.keys(workflow.on ?? {})) !== canonical(["push"]) ||
      canonical(workflow.on?.push?.tags) !== canonical(["wasm/v[0-9]+.[0-9]+.[0-9]+"])) {
    fail("wasm-publish.yml must run only for stable wasm/vX.Y.Z tags");
  }
  if (workflow.concurrency?.["cancel-in-progress"] !== false ||
      !String(workflow.concurrency?.group ?? "").includes("${{ github.ref_name }}")) {
    fail("wasm-publish.yml must serialize idempotent publication attempts by package tag");
  }
  if (canonical(Object.keys(workflow.jobs ?? {}).sort()) !== canonical(["candidate", "publish"])) {
    fail("wasm-publish.yml must contain only candidate and publish jobs");
  }
  const candidate = workflow.jobs.candidate;
  const publish = workflow.jobs.publish;
  if (needs(candidate).length !== 0 || canonical(needs(publish)) !== canonical(["candidate"])) {
    fail("wasm-publish.yml must verify one candidate before publication");
  }
  if (publish.environment !== "npm-publish") {
    fail("wasm-publish.yml must keep Trusted Publishing in the npm-publish environment");
  }
  const checkout = (candidate.steps ?? []).find((step) =>
    typeof step.uses === "string" && step.uses.startsWith("actions/checkout@")
  );
  if (checkout?.with?.["fetch-depth"] !== 0 || checkout?.with?.["fetch-tags"] !== true ||
      checkout?.with?.["persist-credentials"] !== false) {
    fail("wasm-publish.yml must fetch tag and main history without checkout credentials");
  }
  const candidateSource = jobRuns(candidate);
  for (const required of [
    "packages/wasm/package.json",
    'test "$PACKAGE_TAG" = "wasm/v$version"',
    'git rev-parse "refs/tags/$PACKAGE_TAG^{commit}"',
    "git rev-parse HEAD",
    'git merge-base --is-ancestor "$PACKAGE_COMMIT" refs/remotes/origin/main',
    "cargo make test-wasm-release-candidate",
  ]) {
    if (!candidateSource.includes(required)) {
      fail(`wasm-publish.yml must bind the package tag, version, source, and candidate: ${required}`);
    }
  }
  const publishSource = jobRuns(publish);
  for (const required of [
    "tools/npm-publication.mjs",
    'npm publish "$tarball"',
    "https://slsa.dev/provenance/v1",
    "nix develop .#ci-browser -c node tools/wasm-npm-smoke.mjs",
  ]) {
    if (!publishSource.includes(required)) {
      fail(`wasm-publish.yml must publish and verify the candidate directly on npm: ${required}`);
    }
  }
  const configuration = JSON.stringify(workflow);
  if (/gh release|releases\/tags|workspace_version|WASM_PACKAGE_VERSION/iu.test(configuration)) {
    fail("wasm-publish.yml must not depend on a native Release or workspace version");
  }
  for (const required of [
    '"audit"',
    '"signatures"',
    '"--include-attestations"',
    "runWasmPackageBrowserSmoke",
  ]) {
    if (!npmSmokeSource?.includes(required)) {
      fail(`wasm npm smoke must verify signatures, provenance, and the browser package: ${required}`);
    }
  }
}

export function validateVscodePublication(workflows) {
  const workflow = workflows["vscode-publish.yml"];
  if (!workflow) fail("vscode-publish.yml is required");
  if (workflows["marketplace-publish.yml"] || workflows["open-vsx-publish.yml"]) {
    fail("legacy VS Code publication workflows must not remain");
  }
  if (canonical(Object.keys(workflow.on ?? {})) !== canonical(["push"]) ||
      canonical(workflow.on?.push?.tags) !== canonical(["vscode/v[0-9]+.[0-9]+.[0-9]+"])) {
    fail("vscode-publish.yml must run only for stable vscode/vX.Y.Z tags");
  }
  if (workflow.concurrency?.["cancel-in-progress"] !== false ||
      !String(workflow.concurrency?.group ?? "").includes("${{ github.ref_name }}")) {
    fail("vscode-publish.yml must serialize idempotent publication attempts by extension tag");
  }
  if (canonical(Object.keys(workflow.jobs ?? {}).sort()) !==
      canonical(["candidate", "marketplace", "open-vsx"])) {
    fail("vscode-publish.yml must contain one candidate and two registry jobs");
  }

  const candidate = workflow.jobs.candidate;
  const marketplace = workflow.jobs.marketplace;
  const openVsx = workflow.jobs["open-vsx"];
  if (needs(candidate).length !== 0 || canonical(needs(marketplace)) !== canonical(["candidate"]) ||
      canonical(needs(openVsx)) !== canonical(["candidate"])) {
    fail("both VS Code registry jobs must consume the same verified candidate");
  }
  const checkout = (candidate.steps ?? []).find((step) =>
    typeof step.uses === "string" && step.uses.startsWith("actions/checkout@")
  );
  if (checkout?.with?.["fetch-depth"] !== 0 || checkout?.with?.["fetch-tags"] !== true ||
      checkout?.with?.["persist-credentials"] !== false) {
    fail("vscode-publish.yml must fetch tag and main history without checkout credentials");
  }
  const candidateSource = jobRuns(candidate);
  for (const required of [
    "editors/vscode/package.json",
    'test "$PACKAGE_TAG" = "vscode/v$version"',
    'git rev-parse "refs/tags/$PACKAGE_TAG^{commit}"',
    'test "$tag_commit" = "$PACKAGE_COMMIT"',
    'git merge-base --is-ancestor "$PACKAGE_COMMIT" refs/remotes/origin/main',
    "cargo make test-vscode-release-candidate",
  ]) {
    if (!candidateSource.includes(required)) {
      fail(`vscode-publish.yml must bind the extension tag, version, source, and candidate: ${required}`);
    }
  }
  const upload = (candidate.steps ?? []).find((step) =>
    typeof step.uses === "string" && step.uses.startsWith("actions/upload-artifact@")
  );
  if (upload?.with?.name !== "vscode-extension-candidate") {
    fail("vscode-publish.yml must upload the verified VSIX exactly once");
  }
  for (const [name, job] of [["marketplace", marketplace], ["open-vsx", openVsx]]) {
    const download = (job.steps ?? []).find((step) =>
      typeof step.uses === "string" && step.uses.startsWith("actions/download-artifact@")
    );
    if (download?.with?.name !== upload.with.name) {
      fail(`vscode-publish.yml ${name} must consume the verified VSIX artifact`);
    }
    const source = jobRuns(job);
    for (const required of ["--name adocweave", "--version \"$VERSION\"", "--output", "test-vscode-published-extension"]) {
      if (!source.includes(required)) {
        fail(`vscode-publish.yml ${name} must retrieve and run the published extension: ${required}`);
      }
    }
    if (!source.includes("steps.existing.outputs.state") &&
        !(job.steps ?? []).some((step) => String(step.if ?? "").includes("steps.existing.outputs.state == 'missing'"))) {
      fail(`vscode-publish.yml ${name} must skip an identical existing publication`);
    }
  }

  if (marketplace.environment !== "marketplace-publish" ||
      openVsx.environment !== "open-vsx-publish") {
    fail("VS Code registry credentials must use separate GitHub environments");
  }
  const marketplaceSource = jobRuns(marketplace);
  if (!marketplaceSource.includes("vsce publish") || !marketplaceSource.includes("--oidc") ||
      /--azure-credential|AZURE_CLIENT_ID|AZURE_TENANT_ID|Azure\/login|VSCE_PAT/iu.test(
        JSON.stringify(marketplace),
      )) {
    fail("Marketplace publication must use only vsce trusted publishing with OIDC");
  }
  const openVsxSource = jobRuns(openVsx);
  if (!openVsxSource.includes("ovsx publish") ||
      !JSON.stringify(openVsx).includes("${{ secrets.OPEN_VSX_TOKEN }}")) {
    fail("Open VSX publication must use only its environment token and ovsx");
  }
  const openVsxTokenUsers = [];
  for (const [workflowName, candidateWorkflow] of Object.entries(workflows)) {
    const workflowScope = { ...candidateWorkflow };
    delete workflowScope.jobs;
    if (JSON.stringify(workflowScope).includes("OPEN_VSX_TOKEN")) {
      openVsxTokenUsers.push(`${workflowName}:workflow`);
    }
    for (const [jobName, job] of Object.entries(candidateWorkflow.jobs ?? {})) {
      if (JSON.stringify(job).includes("OPEN_VSX_TOKEN")) {
        openVsxTokenUsers.push(`${workflowName}:${jobName}`);
      }
    }
  }
  if (canonical(openVsxTokenUsers) !== canonical(["vscode-publish.yml:open-vsx"])) {
    fail("OPEN_VSX_TOKEN must be used only by vscode-publish.yml open-vsx");
  }
  if (/gh release|releases\/tags|workspace_version|adocweave-vscode/iu.test(JSON.stringify(workflow))) {
    fail("vscode-publish.yml must not depend on a native Release, workspace version, or old ID");
  }
}

export function validateReleaseVersionCommands(source) {
  const invocations = [...source.matchAll(/node tools\/sync-release-version\.mjs ([^\n]+)/g)]
    .map((match) => match[1].trim().split(/\s+/));
  if (invocations.length === 0) fail("release guide must show sync-release-version commands");
  for (const arguments_ of invocations) parseReleaseVersionArguments(arguments_);
  if (!invocations.some((arguments_) => arguments_[0] === "--check") ||
      !invocations.some((arguments_) => arguments_[0] === "--version")) {
    fail("release guide must show both --check and --version");
  }
}

export function loadWorkflowPolicyInputs() {
  const sources = {};
  for (const file of readdirSync(new URL(".github/workflows/", ROOT))) {
    if (file.endsWith(".yml")) sources[file] = read(`.github/workflows/${file}`);
  }
  return {
    distConfiguration: read("dist-workspace.toml"),
    releaseGuide: read("CONTRIBUTING.adoc"),
    textlintNpmSmoke: read("tools/textlint-plugin-npm-smoke.mjs"),
    wasmNpmSmoke: read("tools/wasm-npm-smoke.mjs"),
    workflows: Object.fromEntries(
      Object.entries(sources).map(([name, source]) => [name, parseWorkflow(name, source)]),
    ),
  };
}

export function validateReleaseWorkflowPolicy({
  workflows,
  distConfiguration,
  releaseGuide,
  textlintNpmSmoke,
  wasmNpmSmoke,
}) {
  validatePinnedActions(workflows);
  validatePermissions(workflows);
  validateReleaseFlow(workflows, distConfiguration);
  validateCiGates(workflows);
  validateExternalPublicationIsolation(workflows);
  validateBinaryCachePublication(workflows);
  validateTextlintPluginPublication(workflows, textlintNpmSmoke);
  validateWasmPublication(workflows, wasmNpmSmoke);
  validateVscodePublication(workflows);
  validateReleaseVersionCommands(releaseGuide);
}

export function main() {
  validateReleaseWorkflowPolicy(loadWorkflowPolicyInputs());
  process.stdout.write("release workflow policy verified\n");
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
