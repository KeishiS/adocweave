import { readFileSync } from "node:fs";

import { TEXTLINT_ADAPTER_API_VERSION } from "../packages/textlint-plugin-asciidoc/bridge.mjs";
import {
  PROTOCOL_SCHEMA_VERSION,
  WORKER_PROTOCOL_VERSION,
} from "../web-worker/worker-protocol.mjs";

const ROOT = new URL("../", import.meta.url);
const plan = JSON.parse(readFileSync(new URL("release/distribution-plan.json", ROOT), "utf8"));

function fail(message) {
  throw new Error(message);
}

function readVersionSource(versionSource) {
  const [path, selector, ...rest] = versionSource.split("#");
  if (!path || !selector || rest.length > 0) fail(`versionSourceが不正です：${versionSource}`);
  const source = readFileSync(new URL(path, ROOT), "utf8");
  if (path.endsWith(".json")) {
    const value = selector.split(".").reduce((current, key) => current?.[key], JSON.parse(source));
    return value ?? fail(`versionSourceに値がありません：${versionSource}`);
  }
  if (path.endsWith(".toml")) {
    const keys = selector.split(".");
    const field = keys.pop();
    const section = keys.length > 0 ? `[${keys.join(".")}]` : null;
    const sectionOffset = section ? source.indexOf(section) : 0;
    if (sectionOffset < 0) fail(`versionSourceにsectionがありません：${versionSource}`);
    const scoped = source.slice(sectionOffset + (section?.length ?? 0));
    const match = new RegExp(`^${field}\\s*=\\s*\"([^\"]+)\"`, "m").exec(scoped);
    return match?.[1] ?? fail(`versionSourceに値がありません：${versionSource}`);
  }
  fail(`未対応のversionSourceです：${versionSource}`);
}

export const PRODUCT_IDS = plan.products.map(({ product }) => product);

export function productRelease(product) {
  const route = plan.products.find((candidate) => candidate.product === product);
  if (!route) fail(`未知の製品です：${product}`);
  const version = readVersionSource(route.versionSource);
  if (!/^\d+\.\d+\.\d+$/.test(version)) fail(`${product}の製品バージョンが不正です：${version}`);
  return { product, version, route };
}

export function relatedApiVersions(product) {
  if (product === "lsp") return [];
  if (product === "browser") {
    return [
      { name: "WASM protocol schema", version: PROTOCOL_SCHEMA_VERSION },
      { name: "Worker protocol", version: WORKER_PROTOCOL_VERSION },
    ];
  }
  if (product === "textlint") {
    return [{ name: "textlint adapter API", version: TEXTLINT_ADAPTER_API_VERSION }];
  }
  return [];
}

export const PUBLIC_PROTOCOL_SCHEMA_VERSION = PROTOCOL_SCHEMA_VERSION;
