#!/usr/bin/env node

import { format } from "node:util";

import { processRequest } from "./index.mjs";
import { LIMITS, RequestError, diagnostic, emptyResponse, encodeResponse } from "./protocol.mjs";

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
