import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

// This policy checks what other gates cannot: supply-chain pinning,
// write-permission scope, direct secret access, and product-specific release
// routing. Workflow syntax is actionlint's job (the `workflow-lint` task).

const ROOT = new URL("../", import.meta.url);
const read = (path) => readFileSync(new URL(path, ROOT), "utf8");
const PUBLIC_TASKS = [
  "acceptance", "default", "fmt", "fmt-check", "main-gate", "release-check",
  "security-audit", "test-browser-release-candidate", "test-global-product-candidate",
  "test-vscode-release-determinism", "test-zed-release-candidate",
  "textlint-plugin-post-release-npx-smoke", "textlint-plugin-release-consumer-e2e", "verify",
].sort();
const DEVELOPER_TASKS = new Set([
  "acceptance", "fmt", "fmt-check", "release-check", "security-audit", "verify",
]);

// The only places allowed to hold a write permission. Publication needs
// `contents: write` for the GitHub Release and `id-token: write` for artifact
// attestation; every other workflow and job stays read-only.
const PUBLISH_PERMISSIONS = {
  attestations: "write",
  contents: "write",
  "id-token": "write",
};
const ALLOWED_WRITE_GRANTS = new Set([
  "release.yml job publish-native",
  "release.yml job publish-global",
  "release-publish.yml job publish",
]);

// Workflows use the ambient job token only. The exceptions are the two
// publications after a stable release that no OIDC federation covers: the
// binary cache push reads the Cachix write token, and the Open VSX publication
// reads its registry token. Both run after the GitHub Release exists and hold no
// write permission. No other job, workflow, or secret is allowed.
const ALLOWED_SECRET_REFERENCES = new Map([
  ["binary-cache-publish.yml job publish", new Set(["secrets.CACHIX_AUTH_TOKEN"])],
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
  const canonicalPermissions = (permissions) => JSON.stringify(
    Object.fromEntries(Object.entries(permissions ?? {}).sort(([left], [right]) => left.localeCompare(right))),
  );
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
      const location = `${name} job ${jobName}`;
      grants(job.permissions, location);
      if (ALLOWED_WRITE_GRANTS.has(location) &&
          canonicalPermissions(job.permissions) !== canonicalPermissions(PUBLISH_PERMISSIONS)) {
        fail(`${location} must grant exactly the publication permissions`);
      }
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
  const release = workflows["release.yml"];
  const publish = workflows["release-publish.yml"];
  if (workflows["release-dispatch.yml"] !== undefined) {
    fail("過去runを再利用するrelease-dispatch workflowを残さないでください");
  }
  const productInput = release?.on?.workflow_dispatch?.inputs?.product;
  if (
    productInput?.required !== true ||
    productInput.type !== "choice" ||
    JSON.stringify(productInput.options) !== JSON.stringify(RELEASE_PRODUCTS)
  ) {
    fail("release workflow dispatch must require one supported product");
  }
  const planOutputs = release.jobs?.["candidate-plan"]?.outputs ?? {};
  if (planOutputs.product !== "${{ steps.plan.outputs.product }}") {
    fail("candidate plan must expose the selected product");
  }
  for (const [jobName, dependency] of [
    ["publish-native", "installation-e2e"],
    ["publish-global", "verify-global-candidate"],
  ]) {
    const job = release.jobs?.[jobName];
    if (job?.uses !== "./.github/workflows/release-publish.yml" ||
        job.with?.product !== "${{ needs.candidate-plan.outputs.product }}" ||
        !needs(job).includes(dependency)) {
      fail(`${jobName} must publish only the verified selected product`);
    }
  }
  for (const [job, product] of [
    ["textlint-plugin-post-release-smoke", "textlint"],
    ["open-vsx", "vscode"],
    ["binary-cache", "cli"],
  ]) {
    const condition = release.jobs?.[job]?.if;
    if (typeof condition !== "string" || !condition.includes(`needs.candidate-plan.outputs.product == '${product}'`)) {
      fail(`release job ${job} must run only for ${product}`);
    }
    const expectedPublish = product === "cli" ? "publish-native" : "publish-global";
    if (!needs(release.jobs[job]).includes(expectedPublish)) {
      fail(`release job ${job} must wait for ${expectedPublish}`);
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
  if (publish.jobs?.publish?.environment !== "github-release" ||
      typeof verification !== "string" ||
      !verification.includes("--verify-publication \"$PRODUCT\"") ||
      !verification.includes('verify "$PRODUCT" artifacts "$GITHUB_SHA"')) {
    fail("release publish must verify the normalized product publication plan");
  }
  const steps = publish.jobs.publish.steps ?? [];
  const index = (name) => steps.findIndex((step) => step.name === name);
  const attest = steps.findIndex((step) => step.uses === "actions/attest@1e69f48acb82d1966a394da916b4c1698aa569d6");
  const order = [
    index("Immutable source tree verification"),
    index("Immutable release input verification"),
    attest,
    index("Immutable stable tag creation"),
    index("Private draft creation and verification"),
    index("Complete release publication"),
  ];
  if (order.some((value) => value < 0) || order.some((value, position) => position > 0 && value <= order[position - 1])) {
    fail("release publish must verify, attest, tag, verify the draft, and publish in order");
  }
  if (steps[attest]?.with?.["subject-path"] !== "artifacts/*" ||
      !steps[order[0]].run?.includes("git status --porcelain --untracked-files=all")) {
    fail("release publish must attest every candidate file from a clean source tree");
  }
  const tagRun = steps[order[3]].run ?? "";
  const finalRun = steps[order[5]].run ?? "";
  if (!tagRun.includes('actual_commit') || !tagRun.includes('= "$GITHUB_SHA"') ||
      !finalRun.includes(".tag_name") || !finalRun.includes(".assets[].name")) {
    fail("release publish must verify the tag commit and final release asset set");
  }
  const draftCleanup = index("Incomplete draft removal");
  const tagCleanup = index("Incomplete stable tag removal");
  const draftCleanupRun = steps[draftCleanup]?.run ?? "";
  const tagCleanupRun = steps[tagCleanup]?.run ?? "";
  if (draftCleanup <= order[5] || tagCleanup <= draftCleanup ||
      !normalizedCondition(steps[draftCleanup]).includes("failure() || cancelled()") ||
      !normalizedCondition(steps[tagCleanup]).includes("failure() || cancelled()") ||
      !draftCleanupRun.includes("adocweave-release-run") || !draftCleanupRun.includes(".draft") ||
      !draftCleanupRun.includes('test "$(jq -r .draft <<<"$draft")" = true') ||
      !draftCleanupRun.includes("contains($marker)") ||
      !tagCleanupRun.includes("matching-refs/tags") || !tagCleanupRun.includes('if [ -n "$existing" ]') ||
      !tagCleanupRun.includes('expected="$RELEASE_TAG_SHA"') ||
      !tagCleanupRun.includes('if [ "$actual" != "$expected" ]') ||
      !tagCleanupRun.includes("GitHub Actions run $GITHUB_RUN_ID") ||
      !tagCleanupRun.includes('test "$(jq -r .message <<<"$tag_object")" = "$expected_message"') ||
      !tagCleanupRun.includes('test "$(jq -r .object.sha <<<"$tag_object")" = "$GITHUB_SHA"') ||
      !tagCleanupRun.includes("git/refs/tags/$RELEASE_TAG")) {
    fail("release cleanup must remove only this run's draft and unchanged unpublished tag");
  }
  for (const workflowName of ["open-vsx-publish.yml", "binary-cache-publish.yml"]) {
    const downstreamSteps = workflows[workflowName]?.jobs?.publish?.steps ?? [];
    const verificationName = workflowName === "open-vsx-publish.yml"
      ? "Published VSIX download and verification"
      : "Published CLI candidate verification";
    const verificationIndex = downstreamSteps.findIndex((step) => step.name === verificationName);
    const secretIndex = downstreamSteps.findIndex((step) => JSON.stringify(step).includes("secrets."));
    const runs = downstreamSteps[verificationIndex]?.run ?? "";
    if (verificationIndex < 0 || secretIndex <= verificationIndex ||
        !runs.includes(".draft") || !runs.includes(".prerelease") || !runs.includes('^{commit}') ||
        !runs.includes("--verify-candidate") || !runs.includes("attestation verify")) {
      fail(`${workflowName} must accept only a published stable release at the checked-out commit`);
    }
  }
  const binaryCacheJob = workflows["binary-cache-publish.yml"]?.jobs?.publish;
  const binaryCacheSystems = (binaryCacheJob?.strategy?.matrix?.include ?? [])
    .map((entry) => entry.nixSystem);
  const binaryCacheRuns = jobRuns(binaryCacheJob);
  if (!["x86_64-linux", "aarch64-linux"].every((system) => binaryCacheSystems.includes(system)) ||
      !binaryCacheRuns.includes('nix build ".#checks.${NIX_SYSTEM}.default"') ||
      !binaryCacheRuns.includes('readlink -f "$check/package"') ||
      !binaryCacheRuns.includes('cachix push keishis "$package"')) {
    fail("binary-cache-publish.yml must check and publish the default package for both Linux architectures");
  }
  const source = JSON.stringify(workflows);
  for (const forbidden of ["candidate_sha", "run-id", "github-token", "actions/workflows/"]) {
    if (source.includes(forbidden)) fail(`release workflows must not use cross-run input: ${forbidden}`);
  }
}

function jobRuns(job) {
  return (job?.steps ?? [])
    .map((step) => step.run)
    .filter((run) => typeof run === "string")
    .join("\n");
}

function hasOnlyRun(job, expected) {
  const commands = (job?.steps ?? [])
    .map((step) => step.run)
    .filter((run) => typeof run === "string")
    .map((run) => run.trim().replace(/\s+/g, " "));
  return commands.length === 1 && commands[0] === expected;
}

function needs(job) {
  if (typeof job?.needs === "string") return [job.needs];
  return Array.isArray(job?.needs) ? job.needs : [];
}

function normalizedCondition(job) {
  const condition = job?.if;
  if (typeof condition !== "string") return "";
  return condition
    .replace(/^\s*\$\{\{\s*/, "")
    .replace(/\s*\}\}\s*$/, "")
    .trim()
    .replace(/\s+/g, " ");
}

function hasMainGateCondition(job) {
  return normalizedCondition(job) === "(github.event_name == 'push' || github.event_name == 'workflow_dispatch') && github.ref == 'refs/heads/main'";
}

function hasDispatchMainCondition(job) {
  return normalizedCondition(job) === "github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/main'";
}

function isMainOnly(jobs, jobName, visiting = new Set()) {
  if (visiting.has(jobName)) return false;
  const job = jobs[jobName];
  if (!job) return false;
  if (hasDispatchMainCondition(job)) return true;
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
  "doc-check",
  "docs-check",
  "fmt-check",
  "html5-check",
  "platform-contract",
  "release-ci-contract",
  "test",
  "test-browser-types",
  "test-vscode",
  "test-web-worker",
  "test-zed",
  "textlint-plugin-public-js-unit",
  "textlint-repository-prose-lint",
  "zed-query-contract",
].sort();

const MAIN_GATE_DEPENDENCIES = [
  "check-zed-wasm",
  "cross-native-check",
  "fuzz-smoke",
  "nix-package-check",
  "protocol-wasm-corpus-check",
  "test-cross-runtime",
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
    ["verify", SOURCE_GATE_DEPENDENCIES],
    ["main-gate", MAIN_GATE_DEPENDENCIES],
  ]) {
    const dependencies = new Set(makeTaskDependencies(makefile, name));
    const missing = expected.filter((dependency) => !dependencies.has(dependency));
    if (missing.length > 0) {
      fail(`${name}に必須の検査依存がありません: ${missing.join(", ")}`);
    }
  }
  const mainDependencies = new Set(makeTaskDependencies(makefile, "main-gate"));
  for (const candidate of ["release-global-candidate", "release-global-artifacts", "wasm-size"]) {
    if (mainDependencies.has(candidate)) {
      fail(`main-gateへ配布成果物task ${candidate}を含めないでください`);
    }
  }
  if (/^\s*alias\s*=/m.test(makeTaskBody(makefile, "verify"))) {
    fail("verifyは別名ではなくsource検査の実体にしてください");
  }
  const acceptance = makeTaskDependencies(makefile, "acceptance");
  if (JSON.stringify(acceptance) !== JSON.stringify(["main-gate", "release-global-candidate"])) {
    fail("acceptanceはsource検査を繰り返さず、main検査と完成candidateだけを実行してください");
  }
  const release = makeTaskDependencies(makefile, "release-check");
  if (JSON.stringify(release) !== JSON.stringify(["acceptance", "dist-plan", "security-audit", "verify", "wasm-size"])) {
    fail("release-checkはverify、security-audit、acceptance、dist-plan、wasm-sizeを合成してください");
  }

  const publicTasks = [...makefile.matchAll(/^\[tasks\.([^\]]+)\]$/gm)]
    .map(([, name]) => name)
    .filter((name) => !/^\s*private\s*=\s*true\s*$/m.test(makeTaskBody(makefile, name)))
    .sort();
  if (JSON.stringify(publicTasks) !== JSON.stringify(PUBLIC_TASKS)) {
    fail(`外向きtaskを日常入口とCI入口へ限定してください: ${publicTasks.join(", ")}`);
  }
  const aliases = [...makefile.matchAll(/^\s*alias\s*=\s*"([^"]+)"\s*$/gm)].map((match) => match[1]);
  if (JSON.stringify(aliases) !== JSON.stringify(["verify"])) {
    fail("互換aliasを残さず、既定taskからverifyへのaliasだけにしてください");
  }
  for (const removed of [
    "source-gate", "dependency-governance", "dependency-contract", "verify-with-global-candidate", "release-global-artifacts",
    "protocol-generated-check", "test-vscode-release-package",
  ]) {
    if (makefile.includes(`[tasks.${removed}]`)) fail(`削除済みの中間task ${removed}を戻さないでください`);
  }
  if (/(?:^|\n)\s*(?:tar|zip)\s/.test(makefile)) {
    fail("archiveの構築・展開・一覧処理をMakefileへ記述しないでください");
  }
  if (occurrences(makefile, "cargo build -p adocweave-wasm --release --target wasm32-unknown-unknown") !== 1) {
    fail("webとNode.js用の共通WebAssemblyを一度だけcompileしてください");
  }
  const testBody = makeTaskBody(makefile, "test");
  if (occurrences(makefile, "cargo test --workspace --all-features") !== 1 ||
      !testBody.includes("rm -f web-worker/protocol.d.mts") ||
      !testBody.includes("git diff --exit-code -- web-worker/protocol.d.mts")) {
    fail("workspace testの一回の実行でprotocol宣言を再生成して比較してください");
  }
  const vscodeCandidate = makeTaskBody(makefile, "test-vscode-release-determinism");
  if (occurrences(makefile, "npm run package --prefix editors/vscode") !== 1 ||
      !vscodeCandidate.includes("--verify-determinism") ||
      !vscodeCandidate.includes("npm run test:vsix-installation --prefix editors/vscode")) {
    fail("VSIX candidateは二回の再現性build後に同じ成果物を導入検査してください");
  }
}

export function validateBuildReuseContract(sources) {
  const textlintPackage = sources["tools/package-textlint-plugin-release.sh"];
  if (textlintPackage &&
      !/ADOCWEAVE_TEXTLINT_PLUGIN_CARGO_TARGET_DIRECTORY:-target\/textlint-wasm-build/.test(textlintPackage)) {
    fail("textlint candidateはmain検査と同じCargo target directoryを再利用してください");
  }
}

export function validateCargoMakeReferences(sources) {
  for (const [path, source] of Object.entries(sources)) {
    for (const match of source.matchAll(/\bcargo make(?:\s+([a-z0-9][a-z0-9_-]*))?/g)) {
      const task = match[1];
      if (task && !DEVELOPER_TASKS.has(task)) {
        fail(`${path}が開発者向けではないcargo-make task ${task}を案内しています`);
      }
    }
  }
}

function loadCargoMakeReferenceSources() {
  const sources = {
    "CONTRIBUTING.adoc": read("CONTRIBUTING.adoc"),
    "README.adoc": read("README.adoc"),
    ".github/pull_request_template.md": read(".github/pull_request_template.md"),
  };
  const visit = (directory) => {
    for (const entry of readdirSync(new URL(directory, ROOT), { withFileTypes: true })) {
      const path = `${directory}${entry.name}`;
      if (entry.isDirectory()) {
        if (!["node_modules", "target", ".vscode-test"].includes(entry.name)) visit(`${path}/`);
      } else if (/\.(?:adoc|md|mjs|sh)$/.test(entry.name) &&
                 !/\.test\./.test(entry.name) && path !== "tools/release-workflow-policy.mjs") {
        sources[path] = read(path);
      }
    }
  };
  for (const directory of ["docs/", "fuzz/", "tools/"]) visit(directory);
  return sources;
}

const REMOVED_RELEASE_ROUTING = [
  ["native-change-plan", /native-change-plan/],
  ["git diffによるpath判定", /\bgit\s+diff\b/],
  ["Pull Request candidate必要性flag", /\b(?:candidate|preflight)_required\b/],
  ["quality到達可能性input", /\b(?:common_preflight_scheduled|run_(?:rust_source|documents|adapters|dependencies|fuzz|nix_package)|quality_(?:rust_source|documents|adapters|dependencies|fuzz|nix_package))\b/],
  ["到達不能用step", /not reachable/i],
  ["always集約", /\balways\s*\(\s*\)/],
  ["job結果の手動照合", /\bneeds\.[A-Za-z0-9_-]+\.result\b/],
  ["Pull Request用candidate分岐", /(?:artifact_key.{0,40}["']local|product.{0,40}["']pr)/],
];

export function validateStandardSourceAndCandidateGates(workflows, sources = {}) {
  const release = workflows["release.yml"];
  const triggers = release?.on;
  if (triggers?.pull_request === undefined ||
      triggers.pull_request?.paths !== undefined ||
      triggers.pull_request?.["paths-ignore"] !== undefined ||
      !Array.isArray(triggers?.push?.branches) ||
      !triggers.push.branches.includes("main") ||
      triggers.push.paths !== undefined ||
      triggers.push["paths-ignore"] !== undefined ||
      triggers.workflow_dispatch === undefined) {
    fail("release workflowはpath filterなしのPull Request、main push、手動公開でsource gateを実行してください");
  }

  const jobs = release.jobs ?? {};
  const source = jobs.source;
  if (!source || source.name !== "verify" || source.uses !== undefined || source.if !== undefined ||
      needs(source).length !== 0 || source.strategy !== undefined) {
    fail("Pull Requestの必須checkはpath条件のない直接job source（表示名verify）にしてください");
  }
  const workflowRuns = Object.values(jobs).map(jobRuns).join("\n");
  const dispatchGuard = source.steps?.find(
    (step) => step.run?.trim() === 'test "$GITHUB_REF" = refs/heads/main',
  );
  if (occurrences(workflowRuns, "cargo make verify") !== 1 ||
      !jobRuns(source).split("\n").some((run) => run.trim() === "nix develop .#ci -c cargo make verify") ||
      normalizedCondition(dispatchGuard) !== "github.event_name == 'workflow_dispatch'") {
    fail("source jobは利用者と同じverifyを1回だけ直接実行してください");
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
  const security = jobs.security;
  if (!security || security.name !== "security-audit" || !hasMainGateCondition(security) ||
      JSON.stringify(needs(security)) !== JSON.stringify(["source"]) ||
      !hasOnlyRun(security, "nix develop .#ci -c cargo make security-audit")) {
    fail("security jobはsource成功後のmainで最新の依存監査を1回実行してください");
  }
  const mainGateNeeds = new Set(needs(mainGate));
  if (!mainGate || !hasMainGateCondition(mainGate) ||
      mainGateNeeds.size !== 2 || !mainGateNeeds.has("source") || !mainGateNeeds.has("security") ||
      occurrences(workflowRuns, "cargo make main-gate") !== 1 ||
      !hasOnlyRun(mainGate, "nix develop .#ci-fuzz -c cargo make main-gate")) {
    fail("main-gateはsourceとsecurityの成功後にmainで実行してください");
  }
  const candidatePlan = jobs["candidate-plan"];
  const candidatePlanNeeds = new Set(needs(candidatePlan));
  if (!candidatePlan ||
      !hasDispatchMainCondition(candidatePlan) ||
      candidatePlanNeeds.size !== 2 ||
      !candidatePlanNeeds.has("source") ||
      !candidatePlanNeeds.has("main-gate") ||
      !hasOnlyRun(candidatePlan, 'node tools/product-candidate-plan.mjs "$GITHUB_OUTPUT" "${{ inputs.product }}"') ||
      !isMainOnly(jobs, "candidate-plan")) {
    fail("candidate-planは手動公開時にsourceとmain-gateへ依存し、選択製品のcandidate planを1回生成してください");
  }

  const candidateJobs = [
    "build-native",
    "native-smoke",
    "build-global",
    "verify-native-candidate",
    "verify-global-candidate",
    "installation-e2e",
    "publish-native",
    "publish-global",
  ];
  for (const jobName of candidateJobs) {
    const job = jobs[jobName];
    if (!job || !isMainOnly(jobs, jobName) || /\b(?:always|failure|cancelled|success)\s*\(/.test(job.if ?? "")) {
      fail(`成果物job ${jobName}はmain candidate経路だけで実行してください`);
    }
  }
  for (const jobName of Object.keys(jobs)) {
    if (jobName === "candidate-plan" || candidateJobs.includes(jobName) ||
        !/(?:^|-)(?:build|smoke|installation|candidate)(?:-|$)/.test(jobName)) continue;
    if (!isMainOnly(jobs, jobName)) {
      fail(`成果物job ${jobName}はmain candidate経路だけで実行してください`);
    }
  }
  if (!hasOnlyRun(
    jobs["build-global"],
    "nix develop .#ci-browser -c cargo make test-global-product-candidate",
  )) {
    fail("build-globalは製品別の完成candidate taskを1回実行してください");
  }
}

export function loadWorkflowPolicyInputs() {
  const sources = {};
  for (const file of readdirSync(new URL(".github/workflows/", ROOT))) {
    if (file.endsWith(".yml")) sources[file] = read(`.github/workflows/${file}`);
  }
  return {
    makefile: read("Makefile.toml"),
    references: loadCargoMakeReferenceSources(),
    sources,
    workflows: Object.fromEntries(
      Object.entries(sources).map(([name, source]) => [name, parseWorkflow(name, source)]),
    ),
  };
}

export function validateReleaseWorkflowPolicy({ makefile = read("Makefile.toml"), references = {}, sources, workflows }) {
  validatePinnedActions(workflows);
  validateWritePermissionGrants(workflows);
  validateNoDirectSecretAccess(sources, workflows);
  validateProductReleaseRouting(workflows);
  validateStandardSourceAndCandidateGates(workflows, sources);
  validateGateTaskContract(makefile);
  validateCargoMakeReferences(references);
  validateBuildReuseContract(references);
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
