import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const DEFAULT_REGISTRY = "https://registry.npmjs.org";

function integrity(bytes) {
  const digest = createHash("sha512").update(bytes).digest("base64");
  return `sha512-${digest}`;
}

export async function checkNpmPublication({
  candidatePath,
  name,
  version,
  registryUrl = DEFAULT_REGISTRY,
  request = fetch
}) {
  const encodedName = encodeURIComponent(name);
  const encodedVersion = encodeURIComponent(version);
  const url = new URL(`/${encodedName}/${encodedVersion}`, registryUrl);
  const response = await request(url, {
    headers: { accept: "application/json" },
    signal: AbortSignal.timeout(120_000)
  });
  if (response.status === 404) return "missing";
  if (!response.ok) {
    throw new Error(`npm Registry request failed with HTTP ${response.status}`);
  }

  const metadata = await response.json();
  if (metadata.name !== name || metadata.version !== version) {
    throw new Error("npm Registry returned a different package identity");
  }
  const expected = integrity(await readFile(candidatePath));
  if (metadata.dist?.integrity !== expected) {
    throw new Error("npm Registry contains different bytes for this package version");
  }
  return "published";
}

function parseArguments(argv) {
  const options = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const option = argv[index];
    const value = argv[index + 1];
    if (!option?.startsWith("--") || value === undefined) {
      throw new Error("Expected --candidate, --name, and --version");
    }
    options.set(option, value);
  }
  const take = (option) => {
    const value = options.get(option);
    if (value === undefined) throw new Error(`${option} is required`);
    return value;
  };
  return {
    candidatePath: take("--candidate"),
    name: take("--name"),
    version: take("--version"),
    registryUrl: options.get("--registry") ?? DEFAULT_REGISTRY
  };
}

async function main() {
  const state = await checkNpmPublication(parseArguments(process.argv.slice(2)));
  process.stdout.write(`${state}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
