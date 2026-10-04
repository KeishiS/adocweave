import assert from "node:assert/strict";
import test from "node:test";
import CSL from "citeproc";

import { renderCitations } from "./citations.mjs";
import { processRequest } from "./index.mjs";
import { parseInlines } from "./inline.mjs";
import { plainText, researchRequest } from "./fixtures.mjs";

function citationsRequest(style) {
  const request = researchRequest(style);
  for (const scope of Object.values(request.scopes)) scope.equations = [];
  return request;
}

test("later citations update earlier disambiguation and bibliography follows style sort order", async () => {
  const request = citationsRequest();
  const first = structuredClone(request);
  first.scopes.body.citations.length = 1;
  assert.equal(plainText((await processRequest(first)).scopes.body.citations[0].inlines), "(Doe 2024, pp. 12–15)");
  const result = await processRequest(request);
  assert.deepEqual(result.diagnostics, []);
  assert.equal(plainText(result.scopes.body.citations[0].inlines), "(Doe 2024b, pp. 12–15)");
  assert.equal(plainText(result.scopes.body.citations[1].inlines), "(Doe 2024a)");
  assert.deepEqual(result.scopes.body.bibliography.map((entry) => entry.id), ["alpha", "zebra"]);
  assert.equal(plainText(result.scopes.notes.citations[0].inlines), "(Doe 2024)");
  assert.deepEqual(result.scopes.notes.bibliography.map((entry) => entry.id), ["zebra"]);
});

test("numeric bibliography wrappers retain a separator between number and title", async () => {
  const result = await processRequest(citationsRequest("numeric.csl"));
  assert.deepEqual(result.diagnostics, []);
  assert.equal(plainText(result.scopes.body.citations[0].inlines), "[1]");
  assert.equal(plainText(result.scopes.body.citations[1].inlines), "[2]");
  assert.match(plainText(result.scopes.body.bibliography[0].inlines), /1\.\s+Zebra result/);
  assert.equal(plainText(result.scopes.notes.citations[0].inlines), "[1]");
});

test("CSL formatting resets and safe links are represented by finite inline kinds", async () => {
  const request = citationsRequest("decorated.csl");
  request.csl.items[0]["container-title"] = "Plain Journal";
  for (const item of request.csl.items) item.URL = `https://example.org/${item.id}?x=1&y=2`;
  const result = await processRequest(request);
  assert.deepEqual(result.diagnostics, []);
  const kinds = new Set();
  const visit = (nodes) => nodes.forEach((node) => { kinds.add(node.kind); if (node.children) visit(node.children); });
  visit(result.scopes.body.citations[0].inlines);
  assert.ok(["emphasis", "strong", "smallcaps", "normal-emphasis", "normal-strong", "normal-smallcaps"].every((kind) => kinds.has(kind)));
  assert.equal(result.scopes.body.bibliography[0].inlines.find((node) => node.kind === "link").href, "https://example.org/zebra?x=1&y=2");
});

test("missing items fail the corresponding key while other citations remain usable", async () => {
  const request = citationsRequest();
  request.scopes.body.citations[0].items[0].id = "absent";
  const result = await processRequest(request);
  assert.equal(result.scopes.body.citations[0].status, "failed");
  assert.equal(result.scopes.body.citations[1].status, "ok");
  assert.ok(result.diagnostics.some(({ key, code }) => key === "cite1" && code === "missing-csl-item"));
});

test("the CSL HTML converter rejects arbitrary elements, CSS, attributes, and unsafe links", () => {
  for (const html of ["<script>x</script>", "<img src=x>", "<i onclick='alert(1)'>x</i>", "<span style='color:red'>x</span>", "<a href='javascript:alert(1)'>x</a>", "<a href='file:///etc/passwd'>x</a>", "<a href='mailto:person@example.org'>x</a>", "<div class='unexpected'>x</div>"]) {
    assert.throws(() => parseInlines(html, true), /Unsupported CSL HTML/);
  }
  assert.deepEqual(parseInlines("A &amp; B<sup>2</sup><sub>i</sub>"), [{ kind: "text", text: "A & B" }, { kind: "superscript", children: [{ kind: "text", text: "2" }] }, { kind: "subscript", children: [{ kind: "text", text: "i" }] }]);
});

test("prefix, suffix, author suppression, and author-only requests reach citeproc", async () => {
  const request = citationsRequest();
  request.scopes.body.citations = [{ key: "cite1", items: [{ id: "zebra", suppressAuthor: true, prefix: "see ", suffix: ", appendix" }] }];
  const result = await processRequest(request);
  assert.deepEqual(result.diagnostics, []);
  assert.equal(plainText(result.scopes.body.citations[0].inlines), "(see 2024, appendix)");
  request.scopes.body.citations[0].items = [{ id: "zebra", authorOnly: true }];
  assert.equal(plainText((await processRequest(request)).scopes.body.citations[0].inlines), "Doe");
});

test("a later processor update cannot revive a citation with a retained error", () => {
  const original = CSL.Engine;
  let call = 0;
  CSL.Engine = class {
    opt = { xclass: "in-text", development_extensions: {} };
    setOutputFormat() {}
    processCitationCluster() {
      return [{ citation_errors: [] }, ++call === 1
        ? [[0, "<script>unsupported</script>"]]
        : [[0, "corrected citation"], [1, "second citation"]]];
    }
    makeBibliography() { return false; }
  };
  try {
    const request = citationsRequest();
    const result = renderCitations("body", request.scopes.body.citations, request.csl);
    assert.deepEqual(result.results[0], { key: "cite1", status: "failed" });
    assert.equal(result.results[1].status, "ok");
    assert.ok(result.diagnostics.some(({ key, severity }) => key === "cite1" && severity === "error"));
  } finally {
    CSL.Engine = original;
  }
});
