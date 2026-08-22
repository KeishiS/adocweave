import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

// This policy checks what other gates cannot: supply-chain pinning,
// write-permission scope, direct secret access, and product-specific release
// routing. Workflow syntax is actionlint's job (the `workflow-lint` task).

const ROOT = new URL("../", import.meta.url);
const read = (path) => readFileSync(new URL(path, ROOT), "utf8");

// The only places allowed to hold a write permission. Publication needs
// `contents: write` for the GitHub Release and `id-token: write` for artifact
// attestation; every other workflow and job stays read-only.
const ALLOWED_WRITE_GRANTS = new Set([
  "release-publish.yml top-level",
  "release-dispatch.yml job publish",
]);

// Workflows use the ambient job token only. The exceptions are the two
// publications after a stable release that no OIDC federation covers: the
// binary cache push reads the Cachix write token, and the Open VSX publication
// reads its registry token. Both run after the GitHub Release exists and hold no
// write permission. No other job, workflow, or secret is allowed.
const ALLOWED_SECRET_REFERENCES = new Map([
  ["release-dispatch.yml job binary-cache", new Set(["secrets.CACHIX_AUTH_TOKEN"])],
  ["open-vsx-publish.yml job publish", new Set(["secrets.OPEN_VSX_TOKEN"])],
]);
const SECRET_REFERENCE = /secrets\.[A-Za-z_][A-Za-z0-9_]*/g;
const RELEASE_PRODUCTS = ["cli", "lsp", "browser", "textlint", "vscode", "zed"];

function fail(message) {
  throw new Error(message);
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

export function validateWritePermissionGrants(workflows) {
  const grants = (permissions, location) => {
    for (const [scope, level] of Object.entries(permissions ?? {})) {
      if (level !== "read" && level !== "none") {
        if (!ALLOWED_WRITE_GRANTS.has(location)) {
          fail(`${location} grants ${scope}: ${level}; write permissions are reserved for publication`);
        }
      }
    }
  };
  for (const [name, document] of Object.entries(workflows)) {
    if (document.permissions === undefined) {
      fail(`${name} must declare explicit top-level permissions`);
    }
    grants(document.permissions, `${name} top-level`);
    for (const [jobName, job] of Object.entries(document.jobs ?? {})) {
      grants(job.permissions, `${name} job ${jobName}`);
    }
  }
}

export function validateNoDirectSecretAccess(sources, workflows) {
  for (const [name, source] of Object.entries(sources)) {
    const references = source.match(SECRET_REFERENCE) ?? [];
    if (references.length === 0) continue;
    let permitted = 0;
    for (const [jobName, job] of Object.entries(workflows[name]?.jobs ?? {})) {
      const location = `${name} job ${jobName}`;
      const allowed = ALLOWED_SECRET_REFERENCES.get(location);
      for (const reference of JSON.stringify(job).match(SECRET_REFERENCE) ?? []) {
        if (!allowed?.has(reference)) {
          fail(`${location} reads ${reference}; workflows use the ambient job token only`);
        }
        permitted += 1;
      }
    }
    // Every textual reference must be accounted for by an allowed job, so a
    // secret read from a top-level `env:` block or a comment cannot hide.
    if (permitted !== references.length) {
      fail(`${name} reads from the secrets context outside an allowed job`);
    }
  }
}

export function validateProductReleaseRouting(workflows) {
  const dispatch = workflows["release-dispatch.yml"];
  const publish = workflows["release-publish.yml"];
  const productInput = dispatch?.on?.workflow_dispatch?.inputs?.product;
  if (
    productInput?.required !== true ||
    productInput.type !== "choice" ||
    JSON.stringify(productInput.options) !== JSON.stringify(RELEASE_PRODUCTS)
  ) {
    fail("release dispatch must require one supported product");
  }
  const readinessOutputs = dispatch.jobs?.readiness?.outputs ?? {};
  if (readinessOutputs.product !== "${{ steps.readiness.outputs.product }}") {
    fail("release readiness must expose the resolved product");
  }
  const publishProduct = dispatch.jobs?.publish?.with?.product;
  if (publishProduct !== "${{ needs.readiness.outputs.product }}") {
    fail("release publish must receive the resolved product");
  }
  const planRun = dispatch.jobs?.plan?.steps?.find((step) => step.id === "plan")?.run;
  if (
    typeof planRun !== "string" ||
    !planRun.includes("--publication-plan \"$PRODUCT\"") ||
    !planRun.includes('if [ "$build" = cargo-dist ]')
  ) {
    fail("release plan must normalize both cargo-dist and script products");
  }
  for (const [job, product] of [
    ["textlint-plugin-post-release-smoke", "textlint"],
    ["open-vsx", "vscode"],
    ["binary-cache", "cli"],
  ]) {
    const condition = dispatch.jobs?.[job]?.if;
    if (typeof condition !== "string" || !condition.includes(`needs.readiness.outputs.product == '${product}'`)) {
      fail(`release dispatch job ${job} must run only for ${product}`);
    }
  }
  const publishInput = publish?.on?.workflow_call?.inputs?.product;
  if (publishInput?.required !== true || publishInput.type !== "string") {
    fail("release publish must require product input");
  }
  const download = publish.jobs?.publish?.steps?.find(
    (step) => typeof step.uses === "string" && step.uses.includes("actions/download-artifact"),
  );
  if (download?.with?.name !== "release-candidate-${{ inputs.product }}") {
    fail("release publish must download only the selected product candidate");
  }
  const verification = publish.jobs?.publish?.steps?.find(
    (step) => step.name === "Immutable release input verification",
  )?.run;
  if (typeof verification !== "string" || !verification.includes("--verify-publication \"$PRODUCT\"")) {
    fail("release publish must verify the normalized product publication plan");
  }
}

function taskSection(makefile, task) {
  const marker = `[tasks.${task}]`;
  const start = makefile.indexOf(marker);
  if (start < 0) fail(`Makefile.toml is missing task ${task}`);
  const end = makefile.indexOf("\n[tasks.", start + marker.length);
  return makefile.slice(start, end < 0 ? undefined : end);
}

const occurrences = (source, value) => source.split(value).length - 1;

export function validateTextlintReleaseGates(workflows, makefile) {
  const release = workflows["release.yml"];
  if (release?.jobs?.["textlint-plugin-installation-e2e"] !== undefined) {
    fail("release workflow must not duplicate the fixed textlint consumer E2E");
  }
  const globalInstallation = release?.jobs?.["global-installation-e2e"]?.steps
    ?.find((step) => step.name === "Global installation and complete removal")?.run;
  if (typeof globalInstallation !== "string" || /\btextlint\b/.test(globalInstallation)) {
    fail("generic global installation E2E must not include textlint");
  }
  const artifacts = taskSection(makefile, "release-global-artifacts");
  const candidate = taskSection(makefile, "release-global-candidate");
  const hostInstallation = taskSection(makefile, "release-installation-e2e-host");
  if (occurrences(artifacts, '"test-textlint-plugin-release-package"') !== 1 ||
      artifacts.includes("textlint-plugin-reproducibility")) {
    fail("global artifact gate must run only the completed textlint archive verifier");
  }
  if (occurrences(candidate, '"textlint-plugin-release-consumer-e2e"') !== 1 ||
      candidate.includes("textlint-plugin-candidate-npx-smoke")) {
    fail("global candidate gate must run only the fixed textlint consumer E2E");
  }
  if (makefile.includes("[tasks.textlint-plugin-compatibility-probe]") ||
      makefile.includes("[tasks.textlint-plugin-candidate-npx-smoke]")) {
    fail("removed textlint release probes must not return to the task graph");
  }
  if (/\btextlint\b/.test(hostInstallation)) {
    fail("generic host installation E2E must not include textlint");
  }
}

export function loadWorkflowPolicyInputs() {
  const sources = {};
  for (const file of readdirSync(new URL(".github/workflows/", ROOT))) {
    if (file.endsWith(".yml")) sources[file] = read(`.github/workflows/${file}`);
  }
  return {
    makefile: read("Makefile.toml"),
    sources,
    workflows: Object.fromEntries(
      Object.entries(sources).map(([name, source]) => [name, parseWorkflow(name, source)]),
    ),
  };
}

export function validateReleaseWorkflowPolicy({ makefile, sources, workflows }) {
  validatePinnedActions(workflows);
  validateWritePermissionGrants(workflows);
  validateNoDirectSecretAccess(sources, workflows);
  validateProductReleaseRouting(workflows);
  validateTextlintReleaseGates(workflows, makefile);
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
