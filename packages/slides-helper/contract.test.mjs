import assert from "node:assert/strict";
import test from "node:test";
import { validateRequest } from "./protocol.mjs";
import { parseInlines } from "./inline.mjs";
import { fixture } from "./fixtures.mjs";

const contract = JSON.parse(fixture("protocol-contract.json"));
test("the Rust and Node request boundary share contract cases", () => {
  for (const { name, valid, request } of contract.requests) {
    if (valid) assert.doesNotThrow(() => validateRequest(request), name);
    else assert.throws(() => validateRequest(request), undefined, name);
  }
});
test("the Rust and Node normalized inline boundary share node limits", () => {
  for (const { emphasisCount, valid } of contract.richInlines) {
    const convert = () => parseInlines("<em>x</em>".repeat(emphasisCount));
    if (valid) assert.doesNotThrow(convert);
    else assert.throws(convert, /inline tree limit/);
  }
});


test("the Rust and Node inline byte limits include escaping and HTML wrappers", () => {
  const escape = (text) => text.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll('"', "&quot;");
  for (const entry of contract.richBytes) {
    let html;
    if (entry.kind === "links") {
      const base = "https://example.org/";
      const href = base + "x".repeat(entry.hrefBytes - base.length);
      html = `<a href="${href}"></a>`.repeat(entry.count);
    } else {
      html = escape(entry.character.repeat(entry.count));
      if (entry.kind === "emphasis") html = `<em>${html}</em>`;
      if (entry.kind === "strong") html = `<strong>${html}</strong>`;
    }
    if (entry.valid) assert.doesNotThrow(() => parseInlines(html), JSON.stringify(entry));
    else assert.throws(() => parseInlines(html), undefined, JSON.stringify(entry));
  }
});
