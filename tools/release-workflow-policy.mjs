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

function jobRuns(job) {
  return (job?.steps ?? [])
    .map((step) => step.run)
    .filter((run) => typeof run === "string")
    .join("\n");
}

function needs(job) {
  if (typeof job?.needs === "string") return [job.needs];
  return Array.isArray(job?.needs) ? job.needs : [];
}

function hasMainCondition(job) {
  const condition = job?.if;
  return typeof condition === "string" &&
    condition.includes("github.event_name == 'push'") &&
    condition.includes("github.ref == 'refs/heads/main'");
}

function isMainOnly(jobs, jobName, visiting = new Set()) {
  if (visiting.has(jobName)) return false;
  const job = jobs[jobName];
  if (!job) return false;
  if (hasMainCondition(job)) return true;
  visiting.add(jobName);
  const dependencies = needs(job);
  const result = dependencies.some((dependency) =>
    isMainOnly(jobs, dependency, new Set(visiting))
  );
  visiting.delete(jobName);
  return result;
}

const occurrences = (source, value) => source.split(value).length - 1;

const SOURCE_GATE_DEPENDENCIES = [
  "adoc-check",
  "check-vscode",
  "clippy",
  "clippy-zed",
  "dependency-governance",
  "doc-check",
  "docs-check",
  "docs-lint",
  "docs-prose-lint",
  "fmt-check",
  "html5-check",
  "platform-contract",
  "protocol-generated-check",
  "release-ci-contract",
  "test",
  "test-browser-types",
  "test-vscode",
  "test-web-worker",
  "test-zed",
  "textlint-plugin-public-js-unit",
  "zed-query-contract",
].sort();

const MAIN_GATE_DEPENDENCIES = [
  "check-wasm",
  "check-zed-wasm",
  "cross-native-check",
  "fuzz",
  "nix-package-check",
  "protocol-wasm-corpus-check",
  "test-profile-release",
  "test-vscode-extension-host",
  "textlint-plugin-browser-isolation",
  "textlint-plugin-wasm-contract",
].sort();

function makeTaskBody(source, name) {
  const heading = `[tasks.${name}]`;
  const start = source.indexOf(heading);
  if (start < 0) fail(`Makefile.tomlにtask ${name}がありません`);
  const next = source.indexOf("\n[tasks.", start + heading.length);
  return source.slice(start + heading.length, next < 0 ? source.length : next);
}

function makeTaskDependencies(source, name) {
  const body = makeTaskBody(source, name);
  const match = /\bdependencies\s*=\s*\[([\s\S]*?)\]/.exec(body);
  return match ? [...match[1].matchAll(/"([^"]+)"/g)].map((entry) => entry[1]).sort() : [];
}

export function validateGateTaskContract(makefile) {
  for (const [name, expected] of [
    ["source-gate", SOURCE_GATE_DEPENDENCIES],
    ["main-gate", MAIN_GATE_DEPENDENCIES],
  ]) {
    if (JSON.stringify(makeTaskDependencies(makefile, name)) !== JSON.stringify(expected)) {
      fail(`${name}の検査依存が標準契約と一致しません`);
    }
  }
  if (!/^\s*alias\s*=\s*"source-gate"\s*$/m.test(makeTaskBody(makefile, "verify"))) {
    fail("verifyはsource-gateの別名にしてください");
  }
  for (const name of ["acceptance", "release-check"]) {
    const dependencies = new Set(makeTaskDependencies(makefile, name));
    if (!dependencies.has("source-gate") || !dependencies.has("main-gate")) {
      fail(`${name}はsource-gateとmain-gateの両方を含めてください`);
    }
  }
}

const REMOVED_RELEASE_ROUTING = [
  ["native-change-plan", /native-change-plan/],
  ["git diffによるpath判定", /\bgit\s+diff\b/],
  ["Pull Request candidate必要性flag", /\b(?:candidate|preflight)_required\b/],
  ["quality到達可能性input", /\b(?:common_preflight_scheduled|run_(?:rust_source|documents|adapters|dependencies|fuzz|nix_package)|quality_(?:rust_source|documents|adapters|dependencies|fuzz|nix_package))\b/],
  ["到達不能用step", /not reachable/i],
  ["always集約", /\balways\s*\(\s*\)/],
  ["Pull Request用candidate分岐", /(?:artifact_key.{0,40}["']local|product.{0,40}["']pr)/],
];

export function validateStandardSourceAndCandidateGates(workflows, sources = {}) {
  const release = workflows["release.yml"];
  const triggers = release?.on;
  if (triggers?.pull_request === undefined ||
      (triggers.pull_request !== null && Object.keys(triggers.pull_request).length !== 0) ||
      !Array.isArray(triggers?.push?.branches) ||
      !triggers.push.branches.includes("main")) {
    fail("release workflowはpath filterなしのPull Requestとmain pushでsource gateを実行してください");
  }

  const jobs = release.jobs ?? {};
  const source = jobs.source;
  if (!source || source.name !== "verify" || source.uses !== undefined || source.if !== undefined ||
      needs(source).length !== 0 || source.strategy !== undefined) {
    fail("Pull Requestの必須checkはpath条件のない直接job source（表示名verify）にしてください");
  }
  const workflowRuns = Object.values(jobs).map(jobRuns).join("\n");
  if (occurrences(workflowRuns, "cargo make source-gate") !== 1 ||
      !jobRuns(source).includes("cargo make source-gate")) {
    fail("source jobは標準source-gateを1回だけ直接実行してください");
  }

  const workflowSource = sources["release.yml"] ?? JSON.stringify(release);
  for (const [label, pattern] of REMOVED_RELEASE_ROUTING) {
    if (pattern.test(workflowSource)) {
      fail(`release workflowに削除済みの${label}を含めないでください`);
    }
  }
  for (const removedJob of ["changes", "quality", "merge-gate", "preflight", "release-plan"]) {
    if (jobs[removedJob] !== undefined) {
      fail(`release workflowに削除済みjob ${removedJob}を含めないでください`);
    }
  }

  const mainGate = jobs["main-gate"];
  if (!mainGate || !hasMainCondition(mainGate) ||
      occurrences(workflowRuns, "cargo make main-gate") !== 1 ||
      !jobRuns(mainGate).includes("cargo make main-gate")) {
    fail("main-gateはmainへのpushだけで実行してください");
  }
  const candidatePlan = jobs["candidate-plan"];
  const candidatePlanNeeds = new Set(needs(candidatePlan));
  if (!candidatePlan ||
      candidatePlanNeeds.size !== 2 ||
      !candidatePlanNeeds.has("source") ||
      !candidatePlanNeeds.has("main-gate") ||
      occurrences(jobRuns(candidatePlan), "product-candidate-plan.mjs") !== 1 ||
      !isMainOnly(jobs, "candidate-plan")) {
    fail("candidate-planはsourceとmain-gateに依存し、製品別candidate planを1回生成してください");
  }

  const candidateJobs = [
    "build-native",
    "native-smoke",
    "build-global",
    "verify-native-candidate",
    "verify-global-candidate",
    "installation-e2e",
  ];
  for (const jobName of candidateJobs) {
    const job = jobs[jobName];
    if (!job || !isMainOnly(jobs, jobName) || /\b(?:always|failure|cancelled|success)\s*\(/.test(job.if ?? "")) {
      fail(`成果物job ${jobName}はmain candidate経路だけで実行してください`);
    }
  }
  const globalCandidateRun = jobRuns(jobs["build-global"]);
  for (const [product, task] of [
    ["browser", "test-browser-release-candidate"],
    ["textlint", "textlint-plugin-release-consumer-e2e"],
    ["vscode", "test-vscode-release-determinism"],
    ["zed", "test-zed-release-candidate"],
  ]) {
    if (!globalCandidateRun.includes(`${product}) task=${task}`)) {
      fail(`build-globalは${product}の完成candidate task ${task}を実行してください`);
    }
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

export function validateReleaseWorkflowPolicy({ makefile = read("Makefile.toml"), sources, workflows }) {
  validatePinnedActions(workflows);
  validateWritePermissionGrants(workflows);
  validateNoDirectSecretAccess(sources, workflows);
  validateProductReleaseRouting(workflows);
  validateStandardSourceAndCandidateGates(workflows, sources);
  validateGateTaskContract(makefile);
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
