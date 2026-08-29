import { readFileSync } from "node:fs";

const ROOT = new URL("../", import.meta.url);
const STABLE_VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

function fail(message) {
  throw new Error(message);
}

export function workspaceVersion(root = ROOT) {
  const manifest = readFileSync(new URL("Cargo.toml", root), "utf8");
  const section = manifest.match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/u)?.[1];
  const version = section?.match(/^version\s*=\s*"([^"]+)"/mu)?.[1];
  if (!version || !STABLE_VERSION.test(version)) fail("workspace version is missing or invalid");
  return version;
}

export function releaseTag(version = workspaceVersion()) {
  if (!STABLE_VERSION.test(version)) fail(`invalid release version: ${version}`);
  return `v${version}`;
}

export function isStableVersion(version) {
  return STABLE_VERSION.test(version);
}
