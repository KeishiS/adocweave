import assert from "node:assert/strict";
import test from "node:test";
import { parseFragment } from "parse5";

import { processRequest } from "./index.mjs";
import { fixture, researchRequest } from "./fixtures.mjs";

function mathRequest(equations, eqnums = "ams") {
  return { schemaVersion: 1, eqnums, scopes: { body: { equations, citations: [] }, notes: { equations: [], citations: [] } } };
}

function svgNodes(svg, predicate) {
  const found = [];
  const visit = (node) => {
    if (predicate(node)) found.push(node);
    for (const child of node.childNodes ?? []) visit(child);
  };
  visit(typeof svg === "string" ? parseFragment(svg) : svg);
  return found;
}
const attribute = (node, name) => node.attrs?.find((entry) => entry.name === name)?.value;
const svgKind = (svg, kind) => svgNodes(svg, (node) => attribute(node, "data-mml-node") === kind);
const labelText = (svg) => svgKind(svg, "mtext").map((node) => svgNodes(node,
  (child) => attribute(child, "data-c") || child.nodeName === "#text")
  .map((child) => child.value ?? String.fromCodePoint(Number.parseInt(attribute(child, "data-c"), 16))).join("")).join("");

test("the complete scope resolves forward references and emits independent local SVG IDs", async () => {
  const result = await processRequest(researchRequest());
  assert.deepEqual(result.diagnostics, []);
  assert.equal(labelText(result.scopes.body.equations[0].svg), "(1)");
  const ids = new Set();
  const all = Object.entries(result.scopes).flatMap(([scope, output]) => output.equations.map((equation) => [scope, equation]));
  for (const [scope, equation] of all) {
    assert.equal(equation.status, "ok");
    assert.match(equation.svg, /<defs>/);
    assert.match(equation.svg, /aria-hidden="true"/);
    assert.deepEqual(Object.keys(equation).sort(), ["key", "status", "svg"]);
    for (const [, id] of equation.svg.matchAll(/ id="([^"]+)"/g)) {
      assert.ok(id.startsWith(`${scope}-${equation.key}-`), id);
      assert.ok(!ids.has(id), id);
      ids.add(id);
    }
  }
  for (const [, equation] of all) {
    for (const [, id] of equation.svg.matchAll(/ href="#([^"]+)"/g)) assert.ok(ids.has(id), id);
    assert.doesNotMatch(equation.svg, /<script|<foreignObject|url\(|https:\/\//);
  }
  assert.equal(labelText(result.scopes.body.equations[2].svg), "(2)");
  assert.equal(svgKind(result.scopes.body.equations[2].svg, "mlabeledtr").length, 1);
  assert.equal(labelText(result.scopes.body.equations[3].svg), "(A)");
  assert.equal(labelText(result.scopes.notes.equations[0].svg), "(1)");
});

test("none, AMS, and all use MathJax numbering rules", async () => {
  const equations = [{ key: "bare", tex: "x=1", display: true }, { key: "env", tex: "\\begin{equation}y=2\\end{equation}", display: true }];
  const outputs = {};
  for (const mode of ["none", "ams", "all"]) outputs[mode] = (await processRequest(mathRequest(equations, mode))).scopes.body.equations;
  assert.equal(labelText(outputs.none[0].svg), "");
  assert.equal(labelText(outputs.none[1].svg), "");
  assert.equal(labelText(outputs.ams[0].svg), "");
  assert.equal(labelText(outputs.ams[1].svg), "(1)");
  assert.equal(labelText(outputs.all[0].svg), "(1)");
  assert.equal(labelText(outputs.all[1].svg), "(2)");
});

test("fractions, roots, operators, matrices, and AMS decorations produce static scientific notation", async () => {
  const response = await processRequest(JSON.parse(fixture("shapes.json")));
  assert.deepEqual(response.diagnostics, []);
  assert.ok(response.scopes.body.equations.every(({ status }) => status === "ok"));
  assert.equal(svgKind(response.scopes.body.equations[0].svg, "msqrt").length, 1);
  assert.equal(svgKind(response.scopes.body.equations[0].svg, "mfrac").length, 1);
  assert.equal(svgKind(response.scopes.body.equations[2].svg, "mtable").length, 1);
  assert.equal(svgKind(response.scopes.body.equations[4].svg, "menclose").length, 1);
});

test("unknown macros, duplicate labels, and unresolved refs produce keyed errors", async () => {
  for (const tex of ["\\doesNotExist", "\\require{html}", "\\href{javascript:alert(1)}{x}", "\\eqref{missing}"]) {
    const result = await processRequest(mathRequest([{ key: "bad", tex, display: true }]));
    assert.deepEqual(result.scopes.body.equations, [{ key: "bad", status: "failed" }]);
    assert.ok(result.diagnostics.some((entry) => entry.scope === "body" && entry.key === "bad" && entry.severity === "error"));
  }
  const duplicate = await processRequest(mathRequest(["x", "y"].map((tex, index) => ({ key: `math${index}`, tex: `\\begin{equation}${tex}\\label{same}\\end{equation}`, display: true }))));
  assert.equal(duplicate.scopes.body.equations[1].status, "failed");
  assert.match(duplicate.diagnostics.find((entry) => entry.key === "math1").message, /multiply defined/);
});

test("macros persist within a scope and do not leak into notes or a new request", async () => {
  const input = mathRequest([
    { key: "define", tex: "\\newcommand{\\secret}[1]{\\mathbf{#1}}", display: false },
    { key: "use", tex: "\\secret{x}", display: false },
  ]);
  input.scopes.notes.equations.push({ key: "use", tex: "\\secret{x}", display: false });
  const result = await processRequest(input);
  assert.equal(result.scopes.body.equations[1].status, "ok");
  assert.equal(result.scopes.notes.equations[0].status, "failed");
  const fresh = await processRequest(mathRequest([{ key: "use", tex: "\\secret{x}", display: false }]));
  assert.equal(fresh.scopes.body.equations[0].status, "failed");
});

test("references to failed equations and their reference chains return keyed failures", async () => {
  const result = await processRequest(mathRequest([
    { key: "first", tex: "\\eqref{second}", display: false },
    { key: "second", tex: "\\begin{equation}\\eqref{third}\\label{second}\\end{equation}", display: true },
    { key: "third", tex: "\\begin{equation}\\eqref{missing}\\label{third}\\end{equation}", display: true },
    { key: "independent", tex: "x=1", display: false },
  ]));
  assert.deepEqual(result.scopes.body.equations.map(({ status }) => status), ["failed", "failed", "failed", "ok"]);
  for (const key of ["first", "second", "third"]) {
    assert.ok(result.diagnostics.some((entry) => entry.key === key && entry.severity === "error"));
  }
});

test("configured argument macros and built-in expansion limits work without custom TeX parsing", async () => {
  const request = mathRequest([{ key: "macro", tex: "\\pair{x}", display: false }]);
  request.macros = [{ name: "pair", definition: "(#1,#2)", arguments: 2, default: "a" }];
  assert.equal((await processRequest(request)).scopes.body.equations[0].status, "ok");
  request.macros = [{ name: "pair", definition: "\\pair{#1}", arguments: 1 }];
  const result = await processRequest(request);
  assert.equal(result.scopes.body.equations[0].status, "failed");
  assert.match(result.diagnostics[0].message, /maximum macro substitution count/i);
});

test("TeX input is not reparsed as HTML or closing delimiters", async () => {
  const result = await processRequest(mathRequest([{ key: "text", tex: "\\text{<script> & 漢字 😀 \\)}", display: false }]));
  assert.equal(result.scopes.body.equations[0].status, "ok");
  assert.doesNotMatch(result.scopes.body.equations[0].svg, /<script>/);
  assert.ok(result.scopes.body.equations[0].svg.includes("漢字"));
});
