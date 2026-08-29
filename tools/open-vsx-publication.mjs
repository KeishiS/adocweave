import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const DEFAULT_REGISTRY = "https://open-vsx.org";

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function requiredString(value, name) {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${name} is missing from the Open VSX response`);
  }
  return value;
}

function registryFileUrl(value, { registry, prefix, name }) {
  const raw = requiredString(value, name);
  const url = new URL(raw);
  if (url.origin !== registry.origin || !url.pathname.startsWith(prefix)) {
    throw new Error(`${name} points outside the requested Open VSX version`);
  }
  return url;
}

async function get(request, url) {
  return request(url, { signal: AbortSignal.timeout(120_000) });
}

export async function checkOpenVsxPublication({
  candidatePath,
  namespace,
  name,
  version,
  registryUrl = DEFAULT_REGISTRY,
  request = fetch
}) {
  const registry = new URL(registryUrl);
  const encoded = [namespace, name, version].map(encodeURIComponent).join("/");
  const metadataUrl = new URL(`/api/${encoded}`, registry);
  const metadataResponse = await get(request, metadataUrl);
  if (metadataResponse.status === 404) return "missing";
  if (!metadataResponse.ok) {
    throw new Error(`Open VSX metadata request failed with HTTP ${metadataResponse.status}`);
  }

  const metadata = await metadataResponse.json();
  if (metadata.namespace !== namespace || metadata.name !== name || metadata.version !== version) {
    throw new Error("Open VSX returned a different extension identity");
  }

  const prefix = `/api/${encoded}/file/`;
  const downloadUrl = registryFileUrl(metadata.files?.download, {
    registry,
    prefix,
    name: "files.download"
  });
  const checksumUrl = registryFileUrl(metadata.files?.sha256, {
    registry,
    prefix,
    name: "files.sha256"
  });

  const candidate = await readFile(candidatePath);
  const expected = sha256(candidate);
  const [downloadResponse, checksumResponse] = await Promise.all([
    get(request, downloadUrl),
    get(request, checksumUrl)
  ]);
  if ([downloadResponse.status, checksumResponse.status].some((status) =>
    status === 202 || status === 404
  )) {
    return "pending";
  }
  if (!downloadResponse.ok) {
    throw new Error(`Open VSX download failed with HTTP ${downloadResponse.status}`);
  }
  if (!checksumResponse.ok) {
    throw new Error(`Open VSX checksum request failed with HTTP ${checksumResponse.status}`);
  }

  const published = Buffer.from(await downloadResponse.arrayBuffer());
  const recorded = (await checksumResponse.text()).trim().toLowerCase();
  if (!/^[0-9a-f]{64}$/u.test(recorded)) {
    throw new Error("Open VSX returned an invalid SHA-256 checksum");
  }
  if (recorded !== expected || sha256(published) !== expected) {
    throw new Error("Open VSX contains different bytes for this extension version");
  }
  return "published";
}

function parseArguments(argv) {
  const options = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const option = argv[index];
    const value = argv[index + 1];
    if (!option?.startsWith("--") || value === undefined) {
      throw new Error("Expected --candidate, --namespace, --name, and --version");
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
    namespace: take("--namespace"),
    name: take("--name"),
    version: take("--version"),
    registryUrl: options.get("--registry") ?? DEFAULT_REGISTRY
  };
}

async function main() {
  const state = await checkOpenVsxPublication(parseArguments(process.argv.slice(2)));
  process.stdout.write(`${state}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
