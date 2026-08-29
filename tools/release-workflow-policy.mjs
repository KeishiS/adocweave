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
const OIDC_PUBLICATION = { "id-token": "write" };
const MARKETPLACE_OIDC_PUBLICATION = { contents: "read", "id-token": "write" };
const EXTERNAL_PUBLICATION_WORKFLOWS = new Set([
  "binary-cache-publish.yml",
  "marketplace-publish.yml",
  "npm-publish.yml",
  "open-vsx-publish.yml",
]);
const OIDC_PUBLICATION_WORKFLOWS = new Set([
  "marketplace-publish.yml",
  "npm-publish.yml",
]);

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
      if (OIDC_PUBLICATION_WORKFLOWS.has(name) && jobName === "publish") {
        expectPermissions(
          job.permissions,
          name === "marketplace-publish.yml" ? MARKETPLACE_OIDC_PUBLICATION : OIDC_PUBLICATION,
          `${name} publish permissions`,
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

  const host = jobs.host;
  const required = [
    "plan",
    "build-local-artifacts",
    "build-global-artifacts",
    "custom-native-artifact-smoke",
  ];
  if (!host || !required.every((jobName) => needs(host).includes(jobName))) {
    fail("release host must wait for plan, local, global, and native smoke jobs");
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
    if (canonical(triggerNames) !== canonical(["workflow_call", "workflow_dispatch"])) {
      fail(`${name} must be isolated behind reusable and manual publication triggers`);
    }
    if (workflow.jobs?.publish === undefined || Object.keys(workflow.jobs).length !== 1) {
      fail(`${name} must contain only its isolated publish job`);
    }
  }

  const marketplace = workflows["marketplace-publish.yml"]?.jobs?.publish;
  if (marketplace?.environment !== "marketplace-publish") {
    fail("marketplace-publish.yml must use the marketplace-publish environment");
  }
  for (const [workflowName, workflow] of Object.entries(workflows)) {
    for (const [jobName, job] of Object.entries(workflow.jobs ?? {})) {
      if (
        job.environment === "marketplace-publish" &&
        (workflowName !== "marketplace-publish.yml" || jobName !== "publish")
      ) {
        fail("marketplace-publish environment must be isolated to marketplace-publish.yml");
      }
    }
  }
  const publication = jobRuns(marketplace);
  const marketplaceConfiguration = JSON.stringify(marketplace);
  if (
    !publication.includes("vsce publish") ||
    !publication.includes("--oidc") ||
    /--azure-credential|AZURE_CLIENT_ID|AZURE_TENANT_ID|Azure\/login/iu.test(
      marketplaceConfiguration,
    )
  ) {
    fail("Marketplace publication must use only vsce trusted publishing with OIDC");
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
    workflows: Object.fromEntries(
      Object.entries(sources).map(([name, source]) => [name, parseWorkflow(name, source)]),
    ),
  };
}

export function validateReleaseWorkflowPolicy({ workflows, distConfiguration, releaseGuide }) {
  validatePinnedActions(workflows);
  validatePermissions(workflows);
  validateReleaseFlow(workflows, distConfiguration);
  validateCiGates(workflows);
  validateExternalPublicationIsolation(workflows);
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
