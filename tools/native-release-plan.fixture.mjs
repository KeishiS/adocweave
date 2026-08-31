import {
  NATIVE_TARGETS,
  expectedReleaseAssets,
} from "./native-release-checks.mjs";

export function nativeReleasePlanFixture({ tag, version }) {
  const artifacts = Object.fromEntries(NATIVE_TARGETS.map((target) => {
    const name = `adocweave-${target}.zip`;
    return [name, {
      assets: [{ kind: "executable", name: "adocweave" }],
      checksum: `${name}.sha256`,
      kind: "executable-zip",
    }];
  }));
  artifacts["sha256.sum"] = { kind: "unified-checksum" };
  return {
    announcement_github_body: "## Release Notes\n\n### Changes",
    announcement_is_prerelease: false,
    announcement_tag: tag,
    artifacts,
    dist_version: "0.31.0",
    github_attestations: true,
    github_attestations_phase: "host",
    releases: [{
      app_name: "adocweave",
      app_version: version,
      artifacts: expectedReleaseAssets(),
    }],
  };
}
