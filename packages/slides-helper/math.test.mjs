import assert from "node:assert/strict";
import test from "node:test";
import { parseFragment } from "parse5";

import { processRequest } from "./index.mjs";
import { fixture, researchRequest } from "./fixtures.mjs";

function mathRequest(equations, eqnums = "ams") {
  return { schemaVersion: 3, extensions: [], eqnums, scopes: { body: { equations, citations: [] }, notes: { equations: [], citations: [] } } };
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

test("inline equations retain operators, numbers, and references between visible terms", async () => {
  const result = await processRequest(mathRequest([
    { key: "equals", tex: "z=3", display: false },
    { key: "plus", tex: "z+3", display: false },
    { key: "reference", tex: "z=\\eqref{target}+3", display: false },
    { key: "target", tex: "\\begin{equation}x=1\\label{target}\\end{equation}", display: true },
  ]));
  assert.deepEqual(result.diagnostics, []);
  const equations = result.scopes.body.equations;
  const expected = [
    ["1D467", "3D", "33"],
    ["1D467", "2B", "33"],
    ["1D467", "3D", "28", "31", "29", "2B", "33"],
  ];
  for (const [index, glyphs] of expected.entries()) {
    assert.ok(equations[index].svg);
    assert.deepEqual(svgNodes(equations[index].svg, node => node.nodeName === "use")
      .map(node => attribute(node, "data-c")), glyphs);
  }
  const reference = svgNodes(equations[2].svg, node => node.nodeName === "a");
  assert.equal(reference.length, 1);
  const target = attribute(reference[0], "href").slice(1);
  assert.ok(svgNodes(equations[3].svg, node => attribute(node, "id") === target).length === 1);
});

test("the complete scope resolves forward references and emits independent local SVG IDs", async () => {
  const result = await processRequest(researchRequest());
  assert.deepEqual(result.diagnostics, []);
  assert.equal(labelText(result.scopes.body.equations[0].svg), "(1)");
  const ids = new Set();
  const all = Object.entries(result.scopes).flatMap(([scope, output]) => output.equations.map((equation) => [scope, equation]));
  for (const [scope, equation] of all) {
    assert.ok(equation.svg);
    assert.match(equation.svg, /<defs>/);
    assert.match(equation.svg, /aria-hidden="true"/);
    assert.deepEqual(Object.keys(equation).sort(), ["key", "svg"]);
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
  assert.ok(response.scopes.body.equations.every(({ svg }) => typeof svg === "string"));
  assert.equal(svgKind(response.scopes.body.equations[0].svg, "msqrt").length, 1);
  assert.equal(svgKind(response.scopes.body.equations[0].svg, "mfrac").length, 1);
  assert.equal(svgKind(response.scopes.body.equations[2].svg, "mtable").length, 1);
  assert.equal(svgKind(response.scopes.body.equations[4].svg, "menclose").length, 1);
});

test("unknown macros, duplicate labels, and unresolved refs produce keyed errors", async () => {
  for (const tex of ["\\doesNotExist", "\\require{html}", "\\href{javascript:alert(1)}{x}", "\\eqref{missing}"]) {
    const result = await processRequest(mathRequest([{ key: "bad", tex, display: true }]));
    assert.equal(result.status, "failed");
    assert.ok(result.diagnostics.some((entry) => entry.scope === "body" && entry.key === "bad" && entry.severity === "error"));
  }
  const duplicate = await processRequest(mathRequest(["x", "y"].map((tex, index) => ({ key: `math${index}`, tex: `\\begin{equation}${tex}\\label{same}\\end{equation}`, display: true }))));
  assert.equal(duplicate.status, "failed");
  assert.match(duplicate.diagnostics.find((entry) => entry.key === "math1").message, /multiply defined/);
});

test("macros persist within a scope and do not leak into notes or a new request", async () => {
  const input = mathRequest([
    { key: "define", tex: "\\newcommand{\\secret}[1]{\\mathbf{#1}}", display: false },
    { key: "use", tex: "\\secret{x}", display: false },
  ]);
  input.scopes.notes.equations.push({ key: "use", tex: "\\secret{x}", display: false });
  const result = await processRequest(input);
  assert.ok(!result.diagnostics.some(({ scope }) => scope === "body"));
  assert.equal(result.status, "failed");
  const fresh = await processRequest(mathRequest([{ key: "use", tex: "\\secret{x}", display: false }]));
  assert.equal(fresh.status, "failed");
});

test("a failed reference rejects the request without returning dependent partial results", async () => {
  const result = await processRequest(mathRequest([
    { key: "first", tex: "\\eqref{second}", display: false },
    { key: "second", tex: "\\begin{equation}\\eqref{third}\\label{second}\\end{equation}", display: true },
    { key: "third", tex: "\\begin{equation}\\eqref{missing}\\label{third}\\end{equation}", display: true },
    { key: "independent", tex: "x=1", display: false },
  ]));
  assert.equal(result.status, "failed");
  assert.equal(result.scopes, undefined);
  assert.ok(result.diagnostics.some((entry) => entry.key === "third" && entry.severity === "error"));
});

test("configured argument macros and built-in expansion limits work without custom TeX parsing", async () => {
  const request = mathRequest([{ key: "macro", tex: "\\pair{x}", display: false }]);
  request.macros = [{ name: "pair", definition: "(#1,#2)", arguments: 2, default: "a" }];
  assert.equal((await processRequest(request)).status, "ok");
  request.macros = [{ name: "pair", definition: "\\pair{#1}", arguments: 1 }];
  const result = await processRequest(request);
  assert.equal(result.status, "failed");
  assert.match(result.diagnostics[0].message, /maximum macro substitution count/i);
});

test("TeX input is not reparsed as HTML or closing delimiters", async () => {
  const result = await processRequest(mathRequest([{ key: "text", tex: "\\text{<script> & 漢字 😀 \\)}", display: false }]));
  assert.equal(result.status, "ok");
  assert.doesNotMatch(result.scopes.body.equations[0].svg, /<script>/);
  assert.ok(result.scopes.body.equations[0].svg.includes("漢字"));
});

test("footnote equations are inline and cannot define numbers or labels, including macros", async () => {
  for (const tex of ["x\\label{footnote}", "\\begin{equation}x\\end{equation}", "\\begin{equation}x\\tag{A}\\end{equation}", "\\newcommand{\\named}{x\\label{hidden}}\\named"]) {
    const result = await processRequest(mathRequest([{ key: "footnote", tex, display: false, footnote: true }], "ams"));
    assert.equal(result.status, "failed", tex);
    assert.ok(result.diagnostics.some(({ code }) => code === "footnote-equation-numbering"), tex);
  }
  const display = await processRequest(mathRequest([{ key: "footnote", tex: "x", display: true, footnote: true }]));
  assert.equal(display.status, "failed");
  const reference = await processRequest(mathRequest([
    { key: "body", tex: "\\begin{equation}x\\label{body}\\end{equation}", display: true },
    { key: "footnote", tex: "y+\\eqref{body}", display: false, footnote: true },
  ], "ams"));
  assert.equal(reference.status, "ok", JSON.stringify(reference.diagnostics));
});

test("selected color, cancel, and mathtools extensions produce finite SVG and preserve AMS references", async () => {
  const request = mathRequest([
    { key: "background", tex: String.raw`dX_t = \colorbox{pink}{$f(t,X_t)$}\,dt + \colorbox{lightblue}{$g(t)$}\,dB_t`, display: true },
    { key: "rgb", tex: String.raw`\definecolor{sample}{RGB}{255,128,64}\colorbox{sample}{$x$}`, display: false },
    { key: "frame", tex: String.raw`\definecolor{accent}{RGB}{255,128,64}\fcolorbox{accent}{lightblue}{$x^2+1$}`, display: true },
    { key: "decimal", tex: String.raw`\definecolor{sample}{rgb}{1,.5,0}\textcolor{sample}{x}`, display: false },
    { key: "gray", tex: String.raw`\definecolor{sample}{gray}{.5}\textcolor{sample}{x}`, display: false },
    { key: "cancel", tex: String.raw`\cancel{x}+\bcancel{y}+\xcancel{z}+\cancelto{0}{x}`, display: true },
    { key: "colon", tex: String.raw`a\coloneqq b\eqqcolon c`, display: false },
    { key: "paired", tex: String.raw`\DeclarePairedDelimiter{\abs}{\lvert}{\rvert}\abs*{x}`, display: false },
    { key: "equation", tex: String.raw`\begin{equation}x=1\label{target}\end{equation}`, display: true },
    { key: "reference", tex: String.raw`\eqref{target}`, display: false },
  ]);
  request.extensions = ["mathtools", "cancel", "color"];
  const result = await processRequest(request);
  assert.deepEqual(result.diagnostics, []);
  assert.equal(result.status, "ok");
  const outputs = Object.fromEntries(result.scopes.body.equations.map(({ key, svg }) => [key, svg]));
  const fills = key => svgNodes(outputs[key], node => attribute(node, "fill")).map(node => attribute(node, "fill"));
  assert.ok(fills("background").includes("pink"));
  assert.ok(fills("background").includes("lightblue"));
  assert.ok(fills("rgb").includes("#ff8040"));
  assert.ok(svgNodes(outputs.frame, node => node.nodeName === "rect" && attribute(node, "fill") === "lightblue").length > 0);
  assert.equal(svgNodes(outputs.frame, node => node.nodeName === "polygon" && attribute(node, "fill") === "#ff8040").length, 4);
  assert.doesNotMatch(outputs.frame, /border\s*:/);
  assert.ok(fills("decimal").includes("#ff7f00"));
  assert.ok(fills("gray").includes("#7f7f7f"));
  assert.ok(svgNodes(outputs.cancel, node => node.nodeName === "line").length >= 4);
  assert.ok(svgKind(outputs.colon, "mo").length >= 2);
  assert.equal(labelText(outputs.reference), "(1)");
  for (const svg of Object.values(outputs)) assert.doesNotMatch(svg, /data-bgcolor/);
  request.extensions = ["color", "cancel", "mathtools"];
  assert.deepEqual(await processRequest(request), result, "selected extension order must not affect rendering");
});

test("extensions are explicitly selected and do not leak across requests", async () => {
  for (const [extension, tex] of [
    ["color", String.raw`\colorbox{pink}{$x$}`],
    ["cancel", String.raw`\cancelto{0}{x}`],
    ["mathtools", String.raw`a\coloneqq b\eqqcolon c`],
  ]) {
    const request = mathRequest([{ key: "sample", tex, display: false }]);
    assert.equal((await processRequest(request)).status, "failed", extension);
    request.extensions = [extension];
    assert.equal((await processRequest(request)).status, "ok", extension);
    request.extensions = [];
    assert.equal((await processRequest(request)).status, "failed", extension);
  }
});
