import * as nodeProcessControl from "node:child_process";
import * as nodeFileSystem from "node:fs";
import { tmpdir } from "node:os";
import nodePath from "node:path";
import process from "node:process";
import {
  TEMPORARY_DIRECTORY_REMOVAL_OPTIONS,
  archiveEntries,
  createRuntimeAdapters,
  nativeInstallationLayout,
  isPathInside,
  nativeArtifactFromPlan,
  nativeTargetPlatform,
  validateArchiveEntries,
} from "./native-platform.mjs";
import { workspaceVersion } from "./native-release-version.mjs";

const runtime = createRuntimeAdapters({
  fileSystem: nodeFileSystem,
  processControl: nodeProcessControl,
  time: { clearTimeout, now: Date.now, setTimeout },
  platform: {
    architecture: process.arch,
    environment: process.env,
    os: process.platform,
  },
  pathApi: nodePath,
});
const {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  readlinkSync,
  realpathSync,
  renameSync,
  rmdirSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} = runtime.fileSystem;
const { execFileSync } = runtime.processControl;
const { basename, delimiter, dirname, join, resolve } = runtime.pathApi;

const [candidateArgument, manifestArgument] = process.argv.slice(2);
if (!candidateArgument || process.argv.length > 4) {
  process.stderr.write(
    "usage: node tools/native-release-installation.mjs CANDIDATE_DIRECTORY [DIST_MANIFEST]\n",
  );
  process.exit(2);
}

const targetByHost = new Map([
  ["darwin/arm64", "aarch64-apple-darwin"],
  ["linux/arm64", "aarch64-unknown-linux-musl"],
  ["linux/x64", "x86_64-unknown-linux-musl"],
  ["win32/x64", "x86_64-pc-windows-msvc"],
]);
const target = targetByHost.get(`${runtime.platform.os}/${runtime.platform.architecture}`);
if (!target) throw new Error(`unsupported native installation host: ${runtime.platform.os}/${runtime.platform.architecture}`);
nativeTargetPlatform(target);

const candidate = realpathSync(resolve(candidateArgument));
const version = workspaceVersion();
const manifestSource = manifestArgument
  ? readFileSync(resolve(manifestArgument), "utf8")
  : runtime.platform.environment.DIST_PLAN;
if (!manifestSource) throw new Error("DIST_PLAN or a dist manifest file is required");
const plan = JSON.parse(manifestSource);
const { artifact: nativeArtifact, executable: nativeExecutable } = nativeArtifactFromPlan(plan, target);
if (plan.releases[0].app_version !== version) throw new Error("dist manifest version does not match the native version");

const scratch = mkdtempSync(join(tmpdir(), "adocweave-installation-e2e-"));
const home = join(scratch, "home");
const prefix = join(home, ".local");
const {
  activeMarker,
  binDirectory,
  currentLink,
  versionRoot,
} = nativeInstallationLayout(prefix, version, runtime.pathApi);
const previousRoot = join(prefix, "lib", "adocweave", `${version}-previous-fixture`);

function files(directory) {
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return files(path);
    return [path];
  });
}

function assertInside(root, candidatePath) {
  const resolvedRoot = realpathSync(root);
  const resolvedPath = realpathSync(candidatePath);
  if (!isPathInside(resolvedRoot, resolvedPath, runtime.pathApi)) {
    throw new Error(`${candidatePath} escapes ${root}`);
  }
}

function archive(name) {
  const path = join(candidate, name);
  if (!existsSync(path)) throw new Error(`missing release asset: ${name}`);
  assertInside(candidate, path);
  return path;
}

function extract(path, destination) {
  const listing = execFileSync("unzip", ["-Z1", path], { encoding: "utf8" });
  const entries = archiveEntries(listing);
  if (entries.length === 0) throw new Error(`empty release archive: ${basename(path)}`);
  if (validateArchiveEntries(entries).length > 0) {
    throw new Error(`unsafe or unexpected archive path in ${basename(path)}`);
  }
  mkdirSync(destination, { recursive: true });
  execFileSync("unzip", ["-q", path, "-d", destination]);
  assertInside(destination, destination);
  return destination;
}

function atomicLink(targetPath, linkPath) {
  mkdirSync(dirname(linkPath), { recursive: true });
  const staging = `${linkPath}.new`;
  rmSync(staging, { force: true });
  symlinkSync(targetPath, staging);
  renameSync(staging, linkPath);
}

function command(name, args = []) {
  return execFileSync(name, args, {
    encoding: "utf8",
    env: {
      HOME: home,
      PATH: [binDirectory, runtime.platform.environment.PATH].filter(Boolean).join(delimiter),
      SystemRoot: runtime.platform.environment.SystemRoot,
      XDG_CACHE_HOME: join(home, ".cache"),
      XDG_CONFIG_HOME: join(home, ".config"),
      XDG_DATA_HOME: join(home, ".local", "share"),
    },
  });
}

function activateNative(root, label, executables) {
  for (const executable of executables) {
    if (!existsSync(join(root, "bin", executable))) {
      throw new Error(`cannot activate incomplete native version: ${label}`);
    }
  }
  if (runtime.platform.os === "win32") {
    mkdirSync(binDirectory, { recursive: true });
    for (const executable of executables) {
      const destination = join(binDirectory, executable);
      const staging = `${destination}.new`;
      const backup = `${destination}.previous`;
      copyFileSync(join(root, "bin", executable), staging);
      rmSync(backup, { force: true });
      if (existsSync(destination)) renameSync(destination, backup);
      try {
        renameSync(staging, destination);
      } catch (error) {
        if (existsSync(backup)) renameSync(backup, destination);
        throw error;
      }
      rmSync(backup, { force: true });
    }
  } else {
    atomicLink(root, currentLink);
    for (const executable of executables) {
      const link = join(binDirectory, executable);
      if (!existsSync(link)) atomicLink(join(currentLink, "bin", executable), link);
    }
  }
  const markerStaging = `${activeMarker}.new`;
  writeFileSync(markerStaging, `${label}\n`);
  renameSync(markerStaging, activeMarker);
}

function installNative(artifact, executable) {
  const extracted = extract(
    archive(artifact.name),
    join(scratch, "extract", "native"),
  );
  const destination = join(versionRoot, "bin", executable);
  mkdirSync(dirname(destination), { recursive: true });
  copyFileSync(join(extracted, executable), destination);
  if (runtime.platform.os !== "win32") execFileSync("chmod", ["755", destination]);
}

try {
  mkdirSync(home);
  const before = files(home).map((path) => path.slice(home.length + 1));

  installNative(nativeArtifact, nativeExecutable);
  const executables = [nativeExecutable];
  cpSync(versionRoot, previousRoot, { recursive: true });
  activateNative(versionRoot, version, executables);
  if (runtime.platform.os !== "win32" && readlinkSync(currentLink) !== versionRoot) {
    throw new Error("current version link is not pinned");
  }

  for (const executable of executables) {
    const actual = JSON.parse(command(executable, ["--version", "--json"]));
    if (actual.packageVersion !== version) throw new Error(`${executable} version mismatch`);
  }
  activateNative(previousRoot, `${version}-previous-fixture`, executables);
  if (readFileSync(activeMarker, "utf8") !== `${version}-previous-fixture\n`) {
    throw new Error("native rollback did not select the previous version");
  }
  activateNative(versionRoot, version, executables);
  try {
    activateNative(join(prefix, "lib", "adocweave", "incomplete"), "incomplete", executables);
    throw new Error("incomplete native update was accepted");
  } catch (error) {
    if (error instanceof Error && error.message === "incomplete native update was accepted") {
      throw error;
    }
  }
  if (readFileSync(activeMarker, "utf8") !== `${version}\n`) {
    throw new Error("failed native update changed the active version");
  }
  for (const executable of executables) rmSync(join(binDirectory, executable));
  if (runtime.platform.os !== "win32") rmSync(currentLink);
  rmSync(activeMarker);
  rmSync(versionRoot, { recursive: true });
  rmSync(previousRoot, { recursive: true });
  for (const directory of [
    join(prefix, "share", "adocweave"),
    join(prefix, "share"),
    join(prefix, "lib", "adocweave"),
    join(prefix, "lib"),
    binDirectory,
    prefix,
  ]) {
    if (existsSync(directory) && readdirSync(directory).length === 0) rmdirSync(directory);
  }

  const after = files(home).map((path) => path.slice(home.length + 1));
  if (JSON.stringify(after) !== JSON.stringify(before)) {
    throw new Error(`managed files remain after uninstall: ${after.join(", ")}`);
  }
  process.stdout.write(`native release installation passed: ${version} ${target}\n`);
} finally {
  rmSync(scratch, TEMPORARY_DIRECTORY_REMOVAL_OPTIONS);
}
