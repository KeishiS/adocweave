import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readlink,
  realpath,
  rm,
  symlink,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

const DEFAULT_ROOT = fileURLToPath(new URL("../", import.meta.url));

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  const [candidateArchive] = process.argv.slice(2);
  if (!candidateArchive) {
    process.stderr.write(
      "usage: node tools/verify-textlint-plugin-reproducibility.mjs CANDIDATE_TGZ\n",
    );
    process.exit(2);
  }
  await verifyTextlintPluginReproducibility(candidateArchive);
}

export async function verifyTextlintPluginReproducibility(
  candidateArchive,
  {
    repositoryRoot = DEFAULT_ROOT,
    prepareSource = copyTrackedSource,
    buildPackage = buildTextlintPluginPackage,
    verifyPackage = verifyTextlintPluginPackage,
  } = {},
) {
  const root = await realpath(repositoryRoot);
  const candidate = await realpath(resolve(candidateArchive));
  const candidateBytes = await readFile(candidate);
  const candidateHash = sha256(candidateBytes);
  const scratch = await mkdtemp(join(tmpdir(), "adocweave-textlint-reproducibility-"));
  try {
    const sourceDirectory = join(scratch, "source");
    const outputDirectory = join(scratch, "distrib");
    const cargoTargetDirectory = join(scratch, "cargo-target");
    const npmCacheDirectory = join(scratch, "npm-cache");
    const wasmOutputDirectory = join(scratch, "wasm-output");
    await prepareSource(root, sourceDirectory);
    await buildPackage({
      cargoTargetDirectory,
      npmCacheDirectory,
      outputDirectory,
      sourceDirectory,
      wasmOutputDirectory,
    });
    const manifest = JSON.parse(
      await readFile(join(sourceDirectory, "packages/textlint-plugin-asciidoc/package.json"), "utf8"),
    );
    const rebuiltArchive = join(
      outputDirectory,
      `adocweave-textlint-plugin-asciidoc-${manifest.version}.tgz`,
    );
    const rebuiltBytes = await readFile(rebuiltArchive);
    const rebuiltHash = sha256(rebuiltBytes);
    await verifyPackage({ archive: rebuiltArchive, sourceDirectory });

    if (!candidateBytes.equals(rebuiltBytes)) {
      throw new Error(
        `textlint candidate and clean rebuild differ: ${candidateHash} != ${rebuiltHash}`,
      );
    }
    process.stdout.write(`textlint plugin candidate is reproducible: ${candidateHash}\n`);
    return candidateHash;
  } finally {
    await rm(scratch, { force: true, maxRetries: 5, recursive: true, retryDelay: 100 });
  }
}

async function copyTrackedSource(repositoryRoot, destination) {
  await mkdir(destination, { recursive: true });
  const result = await runProcess("git", ["ls-files", "-z"], { cwd: repositoryRoot });
  if (result.code !== 0) {
    throw new Error(`git ls-files failed:\n${result.stderr}`);
  }
  for (const relativePath of result.stdout.split("\0").filter(Boolean)) {
    const source = join(repositoryRoot, relativePath);
    const target = join(destination, relativePath);
    const metadata = await lstat(source);
    await mkdir(dirname(target), { recursive: true });
    if (metadata.isSymbolicLink()) {
      await symlink(await readlink(source), target);
    } else if (metadata.isFile()) {
      await copyFile(source, target);
    } else {
      throw new Error(`tracked path has an unsupported type: ${relativePath}`);
    }
  }
}

async function buildTextlintPluginPackage({
  cargoTargetDirectory,
  npmCacheDirectory,
  outputDirectory,
  sourceDirectory,
  wasmOutputDirectory,
}) {
  const result = await runProcess("bash", ["tools/package-textlint-plugin-release.sh"], {
    cwd: sourceDirectory,
    env: {
      ...process.env,
      ADOCWEAVE_SOURCE_ROOT: sourceDirectory,
      ADOCWEAVE_TEXTLINT_PLUGIN_CARGO_TARGET_DIRECTORY: cargoTargetDirectory,
      ADOCWEAVE_TEXTLINT_PLUGIN_NPM_CACHE: npmCacheDirectory,
      ADOCWEAVE_TEXTLINT_PLUGIN_OUTPUT_DIRECTORY: outputDirectory,
      ADOCWEAVE_TEXTLINT_PLUGIN_WASM_OUTPUT_DIRECTORY: wasmOutputDirectory,
    },
  });
  if (result.code !== 0) {
    throw new Error(
      `textlint plugin build failed with ${result.code}\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`,
    );
  }
}

async function verifyTextlintPluginPackage({ archive, sourceDirectory }) {
  const result = await runProcess(
    process.execPath,
    ["tools/verify-textlint-plugin-package.mjs", archive],
    { cwd: sourceDirectory },
  );
  if (result.code !== 0) {
    throw new Error(
      `independent textlint package verification failed with ${result.code}\n` +
      `stdout:\n${result.stdout}\nstderr:\n${result.stderr}`,
    );
  }
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function runProcess(command, arguments_, { cwd, env = process.env } = {}) {
  return new Promise((resolveProcess, rejectProcess) => {
    const child = spawn(command, arguments_, { cwd, env, stdio: ["ignore", "pipe", "pipe"] });
    const stdout = [];
    const stderr = [];
    child.stdout.on("data", (chunk) => stdout.push(chunk));
    child.stderr.on("data", (chunk) => stderr.push(chunk));
    child.once("error", rejectProcess);
    child.once("close", (code) => resolveProcess({
      code: code ?? 128,
      stderr: Buffer.concat(stderr).toString("utf8"),
      stdout: Buffer.concat(stdout).toString("utf8"),
    }));
  });
}
