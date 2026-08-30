import path from "node:path";

export const WINDOWS_DLL_ALLOWLIST = new Set([
  "advapi32.dll",
  "bcrypt.dll",
  "bcryptprimitives.dll",
  "crypt32.dll",
  "iphlpapi.dll",
  "kernel32.dll",
  "normaliz.dll",
  "ntdll.dll",
  "ole32.dll",
  "secur32.dll",
  "shell32.dll",
  "user32.dll",
  "userenv.dll",
  "ws2_32.dll",
]);

export const TEMPORARY_DIRECTORY_REMOVAL_OPTIONS = Object.freeze({
  recursive: true,
  force: true,
  maxRetries: 10,
  retryDelay: 100,
});

export function createRuntimeAdapters({ fileSystem, processControl, time, platform, pathApi }) {
  for (const [name, value] of Object.entries({ fileSystem, processControl, time, platform, pathApi })) {
    if (!value || typeof value !== "object") throw new TypeError(`${name} adapter is required`);
  }
  if (typeof platform.os !== "string" || typeof platform.architecture !== "string") {
    throw new TypeError("platform adapter requires os and architecture");
  }
  return Object.freeze({ fileSystem, processControl, time, platform, pathApi });
}

export function pathImplementation(os) {
  return os === "win32" ? path.win32 : path.posix;
}

export function nativeExecutableName(executableSuffix) {
  return `adocweave${executableSuffix}`;
}

export function targetPlatform(target) {
  const architecture = target.startsWith("aarch64-")
    ? "arm64"
    : target.startsWith("x86_64-") ? "x64" : undefined;
  const platform = target.endsWith("-unknown-linux-musl")
    ? { os: "linux", executableSuffix: "", minimumOsVersion: null }
    : target.endsWith("-apple-darwin")
      ? { os: "darwin", executableSuffix: "", minimumOsVersion: "14.0" }
      : target.endsWith("-pc-windows-msvc")
        ? { os: "win32", executableSuffix: ".exe", minimumOsVersion: "10.0.17763" }
        : undefined;
  if (!architecture || !platform) throw new Error(`unsupported native target: ${target}`);
  return Object.freeze({ architecture, archive: "zip", ...platform, target });
}

export function nativeArtifactFromPlan(plan, target) {
  const platform = targetPlatform(target);
  const releases = plan?.releases ?? [];
  if (releases.length !== 1 || releases[0]?.app_name !== "adocweave") {
    throw new Error("dist plan must contain exactly one adocweave release");
  }
  const matches = Object.values(plan?.artifacts ?? {}).filter((artifact) =>
    artifact?.kind === "executable-zip" &&
    artifact.target_triples?.length === 1 &&
    artifact.target_triples[0] === target
  );
  if (matches.length !== 1) {
    throw new Error(`dist plan must contain exactly one native archive for ${target}`);
  }
  const artifact = matches[0];
  const expectedName = `adocweave-${target}.zip`;
  if (artifact.name !== expectedName) {
    throw new Error(`dist plan native archive name mismatch: ${artifact.name}`);
  }
  const executables = artifact.assets?.filter(({ kind }) => kind === "executable") ?? [];
  const executable = nativeExecutableName(platform.executableSuffix);
  if (executables.length !== 1 || executables[0].name !== "adocweave" || executables[0].path !== executable) {
    throw new Error(`dist plan native executable mismatch: ${target}`);
  }
  return Object.freeze({ artifact, executable, platform });
}

export function requiredInstallationAssets(kind, target, version) {
  const names = {
    native: `adocweave-${target}.zip`,
    wasm: `adocweave-wasm-${version}.tgz`,
    textlint: `adocweave-textlint-plugin-asciidoc-${version}.tgz`,
    zed: `adocweave-zed-${version}.tar.xz`,
  };
  if (!Object.hasOwn(names, kind)) throw new Error(`unsupported installation kind: ${kind}`);
  return [names[kind]];
}

export function missingInstallationAssets(available, required) {
  const names = new Set(available);
  return required.filter((name) => !names.has(name));
}

export function installationLayout(prefix, version, pathApi) {
  const nativeRoot = pathApi.join(prefix, "lib", "adocweave");
  const shareRoot = pathApi.join(prefix, "share", "adocweave", version);
  return {
    binDirectory: pathApi.join(prefix, "bin"),
    versionRoot: pathApi.join(nativeRoot, version),
    currentLink: pathApi.join(nativeRoot, "current"),
    activeMarker: pathApi.join(nativeRoot, "active-version"),
    wasmRoot: pathApi.join(shareRoot, "wasm"),
    zedRoot: pathApi.join(shareRoot, "zed"),
  };
}

export function isPathInside(root, candidate, pathApi) {
  const relative = pathApi.relative(root, candidate);
  return relative === "" || (!relative.startsWith(`..${pathApi.sep}`) && relative !== ".." &&
    !pathApi.isAbsolute(relative));
}

export function archiveEntries(listing) {
  return listing.replaceAll("\r\n", "\n").split("\n").filter(Boolean);
}

export function validateArchiveEntries(entries, expectedRoot) {
  return entries.filter((entry) =>
    entry.startsWith("/") ||
    entry.includes("\\") ||
    entry.split("/").includes("..") ||
    (expectedRoot && entry !== `${expectedRoot}/` && !entry.startsWith(`${expectedRoot}/`)));
}

export function importedWindowsDlls(dumpbinOutput) {
  return [...dumpbinOutput.matchAll(/^\s+([A-Za-z0-9_.-]+\.dll)\s*$/gim)]
    .map((match) => match[1].toLowerCase());
}

export function unexpectedWindowsDlls(imported) {
  return imported.filter((name) =>
    !WINDOWS_DLL_ALLOWLIST.has(name) &&
    !name.startsWith("api-ms-win-") &&
    !name.startsWith("ext-ms-win-"));
}

export function unexpectedMacosDependencies(otoolOutput) {
  return otoolOutput.split(/\r?\n/).slice(1).map((line) => line.trim()).filter((line) =>
    line && !line.startsWith("/usr/lib/") && !line.startsWith("/System/Library/"));
}

export function macosMinimumVersion(otoolOutput) {
  return /cmd LC_BUILD_VERSION[\s\S]*?minos ([0-9.]+)/.exec(otoolOutput)?.[1];
}

export function shouldRetryRemoval(error, platform) {
  return platform === "win32" && error instanceof Error &&
    ["EBUSY", "EMFILE", "ENFILE", "ENOTEMPTY", "EPERM"].includes(error.code);
}
