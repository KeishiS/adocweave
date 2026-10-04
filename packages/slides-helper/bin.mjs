#!/usr/bin/env node

import { format } from "node:util";
import { readFileSync } from "node:fs";

import { LIMITS, RequestError, diagnostic, emptyResponse, encodeResponse } from "./protocol.mjs";

// Check before loading MathJax/citeproc so an unsupported runtime gets a useful error.
const required = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8")).engines.node;
// This package declares exactly one minimum version, not a general semver range.
const minimumMatch = /^>=(\d+)\.(\d+)\.(\d+)$/.exec(required);
if (!minimumMatch) {
  process.stderr.write("adocweave-slides-helper has an unsupported engines.node requirement; reinstall @adocweave/slides-helper.\n");
  process.exit(1);
}
const minimum = minimumMatch.slice(1).map(Number);
const actual = process.versions.node.split(".").map(Number);
const older = actual[0] < minimum[0] || (actual[0] === minimum[0]
  && (actual[1] < minimum[1] || (actual[1] === minimum[1] && actual[2] < minimum[2])));
if (older) {
  process.stderr.write(`adocweave-slides-helper requires Node.js ${required}; found ${process.versions.node}. Install a supported Node.js version before retrying. See https://github.com/KeishiS/adocweave/blob/main/docs/user-guide/release-installation.adoc\n`);
  process.exit(1);
}
const { processRequest } = await import("./index.mjs");

async function readRequest() {
  const chunks = [];
  let bytes = 0;
  for await (const chunk of process.stdin) {
    bytes += chunk.length;
    if (bytes > LIMITS.inputBytes) throw new RequestError("input-limit", "The request exceeds the input byte limit.");
    chunks.push(chunk);
  }
  try {
    return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(Buffer.concat(chunks)));
  } catch {
    throw new RequestError("invalid-request", "Expected one UTF-8 JSON request on stdin.");
  }
}

let response;
const logs = [];
const originalConsole = Object.fromEntries(["log", "info", "warn", "error"].map((method) => [method, console[method]]));
for (const method of Object.keys(originalConsole)) {
  console[method] = (...values) => {
    if (logs.length < 64) logs.push(format(...values).slice(0, 2048));
  };
}
try {
  response = await processRequest(await readRequest());
} catch (error) {
  response = emptyResponse();
  response.diagnostics.push(diagnostic(null, null, error.code ?? "processing-error", String(error.message ?? error)));
} finally {
  Object.assign(console, originalConsole);
}
for (const message of logs) response.diagnostics.push(diagnostic(null, null, "library-message", message, "warning"));
const { json, failed } = encodeResponse(response);
process.exitCode = failed ? 1 : 0;
process.stdout.write(`${json}\n`);
