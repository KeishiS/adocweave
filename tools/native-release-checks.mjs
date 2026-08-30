import { readFileSync } from "node:fs";
import process from "node:process";

import {
  checkNativeReleaseVersion,
  releaseTag,
  workspaceVersion,
} from "./native-release-version.mjs";

const ROOT = new URL("../", import.meta.url);
export const NATIVE_TARGETS = [
  "aarch64-apple-darwin",
  "aarch64-unknown-linux-musl",
  "x86_64-pc-windows-msvc",
  "x86_64-unknown-linux-musl",
];

function fail(message) {
  throw new Error(message);
}

function read(path) {
  return readFileSync(new URL(path, ROOT), "utf8");
}

export function validateReleaseTag(tag, version = workspaceVersion()) {
  const expected = releaseTag(version);
  if (tag !== expected) fail(`release tag must be exactly ${expected}`);
  return { tag, version };
}

export function expectedReleaseAssets() {
  return [
    ...NATIVE_TARGETS.flatMap((target) => [
      `adocweave-${target}.zip`,
      `adocweave-${target}.zip.sha256`,
    ]),
    "sha256.sum",
  ].sort();
}

export function expectedPublishedReleaseAssets() {
  return [...expectedReleaseAssets(), "dist-manifest.json"].sort();
}

export function validateDistPlan(plan, tag = releaseTag()) {
  const { version } = validateReleaseTag(tag);
  if (plan.dist_version !== "0.31.0") fail("cargo-dist version mismatch");
  if (plan.announcement_tag !== tag || plan.announcement_is_prerelease !== false) {
    fail("dist plan does not describe the stable workspace release");
  }
  if (plan.releases?.length !== 1) fail("dist plan must contain one release");
  const release = plan.releases[0];
  if (release.app_name !== "adocweave" || release.app_version !== version) {
    fail("dist plan must contain the native adocweave app");
  }
  const actualAssets = [...(release.artifacts ?? [])].sort();
  const expectedAssets = expectedReleaseAssets();
  if (JSON.stringify(actualAssets) !== JSON.stringify(expectedAssets)) {
    fail("dist plan release asset set mismatch");
  }
  for (const target of NATIVE_TARGETS) {
    const name = `adocweave-${target}.zip`;
    const archive = plan.artifacts?.[name];
    const executables = archive?.assets?.filter(({ kind }) => kind === "executable") ?? [];
    if (
      archive?.kind !== "executable-zip" || archive?.checksum !== `${name}.sha256` ||
      executables.length !== 1 || executables[0].name !== "adocweave"
    ) {
      fail(`native archive contract mismatch: ${name}`);
    }
  }
  if (plan.artifacts?.["sha256.sum"]?.kind !== "unified-checksum") {
    fail("dist plan must contain the unified checksum list");
  }
  if (plan.github_attestations !== true || plan.github_attestations_phase !== "host") {
    fail("dist plan must attest every hosted artifact");
  }
  return { tag, version, assets: expectedAssets };
}

export function verifyRepository() {
  const version = checkNativeReleaseVersion();
  const dist = read("dist-workspace.toml");
  for (const required of [
    'packages = ["adocweave"]',
    'checksum = "sha256"',
    'github-attestations = true',
    'github-attestations-phase = "host"',
  ]) {
    if (!dist.includes(required)) fail(`dist configuration is missing: ${required}`);
  }
  if (dist.includes("adocweave-lsp") || dist.includes("distribution-plan")) {
    fail("dist configuration contains legacy product routing");
  }
  return { tag: releaseTag(version), version };
}

export function main(args) {
  const unknown = args.filter((arg) => !arg.startsWith("--tag="));
  if (unknown.length > 0 || args.filter((arg) => arg.startsWith("--tag=")).length > 1) {
    fail("usage: node tools/native-release-checks.mjs [--tag=vX.Y.Z]");
  }
  const release = verifyRepository();
  const tag = args.find((arg) => arg.startsWith("--tag="))?.slice(6);
  if (tag) validateReleaseTag(tag, release.version);
  process.stdout.write(`native release checks passed: ${release.tag}\n`);
}

if (process.argv[1] && import.meta.url === new URL(process.argv[1], "file:").href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
