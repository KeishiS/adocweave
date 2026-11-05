import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cpSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { processRequest } from "./index.mjs";
import { LIMITS, emptyResponse, encodeResponse } from "./protocol.mjs";
import { researchRequest } from "./fixtures.mjs";

const empty = () => ({ schemaVersion: 3, extensions: [], eqnums: "none", scopes: { body: { equations: [], citations: [] }, notes: { equations: [], citations: [] } } });
const run = (input) => spawnSync(process.execPath, [new URL("./bin.mjs", import.meta.url).pathname], { input, encoding: "utf8", maxBuffer: LIMITS.outputBytes + 1024 });

test("the executable reports the installed engine requirement before loading processing libraries", () => {
  for (const version of ["22.11.0", "24.18.9", "24.19.0", "25.0.0"]) {
    const script = `Object.defineProperty(process.versions, 'node', { value: ${JSON.stringify(version)} }); await import(${JSON.stringify(new URL("./bin.mjs", import.meta.url).href)});`;
    const output = spawnSync(process.execPath, ["--input-type=module", "--eval", script], { input: JSON.stringify(empty()), encoding: "utf8" });
    if (version === "22.11.0" || version === "24.18.9") {
      assert.equal(output.status, 1);
      assert.equal(output.stdout, "");
      assert.match(output.stderr, /requires Node\.js >=24\.19\.0/);
      assert.ok(output.stderr.includes(`found ${version}`));
      assert.match(output.stderr, /release-installation\.adoc/);
    } else {
      assert.equal(output.status, 0, output.stderr);
      assert.equal(JSON.parse(output.stdout).schemaVersion, 3);
    }
  }
});

test("an unsupported installed engine range fails before any processing library is required", () => {
  const directory = mkdtempSync(join(tmpdir(), "slides-helper-engine-"));
  try {
    for (const file of ["bin.mjs", "protocol.mjs"]) cpSync(new URL(file, import.meta.url), join(directory, file));
    writeFileSync(join(directory, "package.json"), JSON.stringify({ engines: { node: "^24" } }));
    const output = spawnSync(process.execPath, [join(directory, "bin.mjs")], { input: JSON.stringify(empty()), encoding: "utf8" });
    assert.equal(output.status, 1);
    assert.equal(output.stdout, "");
    assert.match(output.stderr, /unsupported engines\.node requirement; reinstall/);
    assert.ok(!output.stderr.includes("index.mjs"));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("an empty request is one complete JSON response with no stdout or stderr logs", () => {
  const output = run(JSON.stringify(empty()));
  assert.equal(output.status, 0);
  assert.equal(output.stderr, "");
  assert.deepEqual(JSON.parse(output.stdout), { schemaVersion: 3, status: "ok", notices: { math: null, citations: null }, scopes: { body: { equations: [], citations: [], bibliography: [] }, notes: { equations: [], citations: [], bibliography: [] } }, diagnostics: [] });
  assert.equal(output.stdout.split("\n").length, 2);
});

test("schema 1 and 2 requests identify the required protocol version", async () => {
  for (const schemaVersion of [1, 2]) {
    const response = await processRequest({ ...empty(), schemaVersion });
    assert.equal(response.schemaVersion, 3);
    assert.equal(response.status, "failed");
    assert.match(response.diagnostics[0].message, /Unsupported schemaVersion; expected 3/);
  }
});

test("invalid schema, unknown fields, key collisions, and count limits are rejected before libraries run", async () => {
  const cases = [];
  cases.push({ ...empty(), schemaVersion: 1 }, { ...empty(), path: "/etc/passwd" });
  const duplicate = empty();
  duplicate.scopes.body.equations.push({ key: "same", tex: "x", display: false });
  duplicate.scopes.body.citations.push({ key: "same", items: [{ id: "x" }] });
  cases.push(duplicate);
  const tooMany = empty();
  tooMany.scopes.body.equations = Array.from({ length: LIMITS.equations + 1 }, (_, index) => ({ key: `m${index}`, tex: "x", display: false }));
  cases.push(tooMany);
  const oversized = empty();
  oversized.scopes.body.equations.push({ key: "large", tex: "😀".repeat(LIMITS.texBytes / 4 + 1), display: false });
  cases.push(oversized);
  for (const request of cases) {
    const response = await processRequest(request);
    assert.equal(response.status, "failed");
    assert.equal(response.scopes, undefined);
    assert.equal(response.diagnostics[0].severity, "error");
    assert.equal(response.diagnostics[0].key, null);
  }
});

test("the executable rejects invalid UTF-8, multiple JSON documents, and oversized stdin with JSON errors", () => {
  for (const input of [Buffer.from([0xff]), `${JSON.stringify(empty())}\n${JSON.stringify(empty())}`, " ".repeat(LIMITS.inputBytes + 1)]) {
    const output = run(input);
    assert.equal(output.status, 1);
    assert.equal(output.stderr, "");
    assert.equal(JSON.parse(output.stdout).diagnostics[0].severity, "error");
  }
});

test("processing errors return diagnostics without partial results", () => {
  const request = empty();
  request.scopes.body.equations.push({ key: "bad", tex: "\\unknown", display: false });
  const output = run(JSON.stringify(request));
  assert.equal(output.status, 1);
  assert.equal(output.stderr, "");
  const response = JSON.parse(output.stdout);
  assert.equal(response.status, "failed");
  assert.equal(response.scopes, undefined);
  assert.ok(response.diagnostics.some(({ key, scope, severity }) => key === "bad" && scope === "body" && severity === "error"));
});

test("a representative full request passes through the executable without library logs", () => {
  const output = run(JSON.stringify(researchRequest()));
  assert.equal(output.status, 0, output.stderr || output.stdout);
  assert.equal(output.stderr, "");
  const result = JSON.parse(output.stdout);
  assert.equal(result.scopes.body.equations.length, 4);
  assert.equal(result.scopes.notes.citations.length, 1);
  assert.match(result.notices.math.fontAttribution, /Antonis Tsolomitis/);
  assert.match(result.notices.citations.attribution, /Frank Bennett/);
});

test("license notices follow the actual equation and citation input even when only notes use them", async () => {
  const math = empty();
  math.scopes.notes.equations.push({ key: "note", tex: "x", display: false });
  const rendered = await processRequest(math);
  assert.notEqual(rendered.notices.math, null);
  assert.equal(rendered.notices.citations, null);
  const citation = researchRequest();
  for (const scope of Object.values(citation.scopes)) scope.equations = [];
  const response = await processRequest(citation);
  assert.equal(response.notices.math, null);
  assert.match(response.notices.citations.license, /EXHIBIT B/);
  let total = 0;
  for (const notices of [rendered.notices.math, response.notices.citations]) {
    for (const value of Object.values(notices)) {
      assert.ok(Buffer.byteLength(value) <= LIMITS.noticeTextBytes);
      total += Buffer.byteLength(value);
    }
  }
  assert.ok(total <= LIMITS.noticeBytes);
});

test("an oversized rendered response is replaced by a bounded global error", () => {
  const response = emptyResponse();
  response.scopes.body.bibliography.push({ id: "large", inlines: [{ kind: "text", text: "x".repeat(LIMITS.outputBytes) }] });
  const { json, failed } = encodeResponse(response);
  assert.ok(Buffer.byteLength(json) < LIMITS.outputBytes);
  assert.equal(failed, true);
  assert.equal(JSON.parse(json).diagnostics[0].code, "output-limit");
});
