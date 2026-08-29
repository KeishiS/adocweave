import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

import { unzipSync, type Unzipped } from "fflate";

const DEFAULT_MARKETPLACE = "https://marketplace.visualstudio.com";
const MAXIMUM_VSIX_BYTES = 10 * 1024 * 1024;
const CONTENT_TYPES = "[Content_Types].xml";
const ROOT_RELATIONSHIPS = "_rels/.rels";
const SIGNATURE_PREFIX = "_xmlsignatures/";
const SIGNATURE_ORIGIN_RELATIONSHIP =
  "http://schemas.openxmlformats.org/package/2006/relationships/digital-signature/origin";
const SIGNATURE_ORIGIN_CONTENT_TYPE =
  "application/vnd.openxmlformats-package.digital-signature-origin";
const SIGNATURE_XML_CONTENT_TYPE =
  "application/vnd.openxmlformats-package.digital-signature-xmlsignature+xml";

interface ExtensionIdentity {
  name: string;
  publisher: string;
  target: string;
  version: string;
}

interface PublicationOptions extends ExtensionIdentity {
  candidatePath: string;
  marketplaceUrl?: string;
  request?: typeof fetch;
}

function decode(bytes: Uint8Array, name: string): string {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw new Error(`${name} is not valid UTF-8`);
  }
}

function xmlAttributes(tag: string): Map<string, string> {
  const attributes = new Map<string, string>();
  for (const match of tag.matchAll(/([A-Za-z_:][A-Za-z0-9_.:-]*)\s*=\s*(["'])(.*?)\2/gu)) {
    const name = match[1];
    const value = match[3];
    if (name === undefined || value === undefined || attributes.has(name)) {
      throw new Error("VSIX metadata contains invalid XML attributes");
    }
    attributes.set(name, value);
  }
  return attributes;
}

function metadataEntries(
  bytes: Uint8Array,
  element: "Default" | "Override" | "Relationship",
  keyNames: readonly string[],
  defaults: Readonly<Record<string, string>> = {},
): Map<string, string> {
  const entries = new Map<string, string>();
  const source = decode(bytes, element);
  for (const match of source.matchAll(new RegExp(`<${element}\\b[^>]*\\/?\\s*>`, "gu"))) {
    const attributes = xmlAttributes(match[0]);
    const key = keyNames.map((name) => attributes.get(name) ?? defaults[name] ?? "").join("\0");
    if (key.includes("\0\0") || key.startsWith("\0") || key.endsWith("\0")) {
      throw new Error(`VSIX ${element} metadata is incomplete`);
    }
    if (entries.has(key)) throw new Error(`VSIX contains duplicate ${element} metadata`);
    entries.set(key, key);
  }
  return entries;
}

function contentTypes(entries: Unzipped, allowSignature: boolean): Map<string, string> {
  const bytes = entries[CONTENT_TYPES];
  if (bytes === undefined) throw new Error(`VSIX is missing ${CONTENT_TYPES}`);
  const declarations = new Map([
    ...metadataEntries(bytes, "Default", ["Extension", "ContentType"]),
    ...metadataEntries(bytes, "Override", ["PartName", "ContentType"]),
  ]);
  for (const key of [...declarations.keys()]) {
    const [part, type] = key.split("\0");
    const signature =
      (part === "sigs" && type === SIGNATURE_ORIGIN_CONTENT_TYPE) ||
      (part?.startsWith(`/${SIGNATURE_PREFIX}`) && type === SIGNATURE_XML_CONTENT_TYPE);
    if (signature && !allowSignature)
      throw new Error("candidate VSIX already contains signature metadata");
    if (signature) declarations.delete(key);
  }
  return declarations;
}

function relationships(entries: Unzipped, allowSignature: boolean): Map<string, string> {
  const bytes = entries[ROOT_RELATIONSHIPS];
  if (bytes === undefined) return new Map();
  const declarations = metadataEntries(bytes, "Relationship", ["Type", "Target", "TargetMode"], {
    TargetMode: "Internal",
  });
  for (const key of [...declarations.keys()]) {
    const [type] = key.split("\0");
    if (type === SIGNATURE_ORIGIN_RELATIONSHIP && !allowSignature) {
      throw new Error("candidate VSIX already contains a signature relationship");
    }
    if (type === SIGNATURE_ORIGIN_RELATIONSHIP) declarations.delete(key);
  }
  return declarations;
}

function validatedEntries(bytes: Uint8Array, source: string): Unzipped {
  if (bytes.byteLength === 0 || bytes.byteLength > MAXIMUM_VSIX_BYTES) {
    throw new Error(`${source} VSIX has an invalid size`);
  }
  let entries: Unzipped;
  try {
    entries = unzipSync(bytes);
  } catch {
    throw new Error(`${source} is not a valid VSIX archive`);
  }
  for (const name of Object.keys(entries)) {
    const segments = name.split("/");
    if (
      name.startsWith("/") ||
      name.includes("\\") ||
      name.includes("\0") ||
      segments.some((segment) => segment === "." || segment === "..")
    ) {
      throw new Error(`${source} VSIX contains an unsafe entry path: ${name}`);
    }
  }
  return entries;
}

function identity(entries: Unzipped): ExtensionIdentity {
  const packageBytes = entries["extension/package.json"];
  const manifestBytes = entries["extension.vsixmanifest"];
  if (packageBytes === undefined || manifestBytes === undefined) {
    throw new Error("VSIX is missing its extension manifests");
  }
  let manifest: unknown;
  try {
    manifest = JSON.parse(decode(packageBytes, "extension/package.json"));
  } catch {
    throw new Error("VSIX extension/package.json is invalid");
  }
  if (typeof manifest !== "object" || manifest === null) {
    throw new Error("VSIX extension/package.json is invalid");
  }
  const record = manifest as Record<string, unknown>;
  const identityTag = /<Identity\b[^>]*\/?\s*>/u.exec(
    decode(manifestBytes, "extension.vsixmanifest"),
  )?.[0];
  if (identityTag === undefined) throw new Error("VSIX manifest has no Identity element");
  const attributes = xmlAttributes(identityTag);
  const target = attributes.get("TargetPlatform") ?? "universal";
  if (
    typeof record.name !== "string" ||
    typeof record.publisher !== "string" ||
    typeof record.version !== "string" ||
    attributes.get("Id") !== record.name ||
    attributes.get("Publisher") !== record.publisher ||
    attributes.get("Version") !== record.version
  ) {
    throw new Error("VSIX manifests contain different extension identities");
  }
  return { name: record.name, publisher: record.publisher, target, version: record.version };
}

function sameMap(left: Map<string, string>, right: Map<string, string>): boolean {
  return left.size === right.size && [...left.keys()].every((key) => right.has(key));
}

function requireIdentity(
  actual: ExtensionIdentity,
  expected: ExtensionIdentity,
  source: string,
): void {
  if (
    Object.keys(expected).some(
      (key) => actual[key as keyof ExtensionIdentity] !== expected[key as keyof ExtensionIdentity],
    )
  ) {
    throw new Error(`${source} VSIX has a different extension identity, version, or target`);
  }
}

function compareVsix(
  candidateBytes: Uint8Array,
  publishedBytes: Uint8Array,
  expected: ExtensionIdentity,
): void {
  const candidate = validatedEntries(candidateBytes, "candidate");
  const published = validatedEntries(publishedBytes, "published");
  for (const [source, actual] of [
    ["candidate", identity(candidate)],
    ["published", identity(published)],
  ] as const) {
    requireIdentity(actual, expected, source);
  }

  if (!sameMap(contentTypes(candidate, false), contentTypes(published, true))) {
    throw new Error("published VSIX contains different non-signature content types");
  }
  if (!sameMap(relationships(candidate, false), relationships(published, true))) {
    throw new Error("published VSIX contains different non-signature relationships");
  }

  const ignored = (name: string): boolean =>
    name === CONTENT_TYPES || name === ROOT_RELATIONSHIPS || name.startsWith(SIGNATURE_PREFIX);
  const candidateNames = Object.keys(candidate)
    .filter((name) => !ignored(name))
    .sort();
  const publishedNames = Object.keys(published)
    .filter((name) => !ignored(name))
    .sort();
  if (candidateNames.join("\0") !== publishedNames.join("\0")) {
    throw new Error("published VSIX contains a different non-signature file list");
  }
  for (const name of candidateNames) {
    const left = candidate[name];
    const right = published[name];
    if (
      left === undefined ||
      right === undefined ||
      !Buffer.from(left).equals(Buffer.from(right))
    ) {
      throw new Error(`published VSIX contains different content for ${name}`);
    }
  }
}

async function responseBytes(response: Response): Promise<Uint8Array> {
  const length = Number(response.headers.get("content-length"));
  if (Number.isFinite(length) && length > MAXIMUM_VSIX_BYTES) {
    throw new Error("Marketplace VSIX exceeds the download limit");
  }
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (bytes.byteLength > MAXIMUM_VSIX_BYTES) {
    throw new Error("Marketplace VSIX exceeds the download limit");
  }
  return bytes;
}

export async function checkMarketplacePublication({
  candidatePath,
  publisher,
  name,
  version,
  target,
  marketplaceUrl = DEFAULT_MARKETPLACE,
  request = fetch,
}: PublicationOptions): Promise<"missing" | "pending" | "published"> {
  const candidate = new Uint8Array(await readFile(candidatePath));
  const expected = { publisher, name, version, target };
  requireIdentity(identity(validatedEntries(candidate, "candidate")), expected, "candidate");
  const url = new URL(
    `/_apis/public/gallery/publishers/${encodeURIComponent(publisher)}/vsextensions/${[
      name,
      version,
    ]
      .map(encodeURIComponent)
      .join("/")}/vspackage`,
    marketplaceUrl,
  );
  const response = await request(url, { signal: AbortSignal.timeout(120_000) });
  if (response.status === 404) return "missing";
  if (response.status === 202) return "pending";
  if (!response.ok) throw new Error(`Marketplace VSIX request failed with HTTP ${response.status}`);
  compareVsix(candidate, await responseBytes(response), expected);
  return "published";
}

function parseArguments(argv: readonly string[]): PublicationOptions {
  const options = new Map<string, string>();
  for (let index = 0; index < argv.length; index += 2) {
    const option = argv[index];
    const value = argv[index + 1];
    if (!option?.startsWith("--") || value === undefined) {
      throw new Error("Expected named Marketplace publication arguments");
    }
    options.set(option, value);
  }
  const take = (option: string): string => {
    const value = options.get(option);
    if (value === undefined || value.length === 0) throw new Error(`${option} is required`);
    return value;
  };
  return {
    candidatePath: take("--candidate"),
    publisher: take("--publisher"),
    name: take("--name"),
    version: take("--version"),
    target: take("--target"),
    marketplaceUrl: options.get("--marketplace") ?? DEFAULT_MARKETPLACE,
  };
}

async function main(): Promise<void> {
  const state = await checkMarketplacePublication(parseArguments(process.argv.slice(2)));
  process.stdout.write(`${state}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error: unknown) => {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  });
}
