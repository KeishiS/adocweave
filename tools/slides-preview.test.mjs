import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const source = await readFile(new URL("../crates/adocweave/assets/slides/preview.js", import.meta.url), "utf8");
const settle = () => new Promise(resolve => setImmediate(resolve));

function client({ generation = 1, iframe = false, fetch } = {}) {
  const requests = [];
  const display = { textContent: "", hidden: true };
  let reloads = 0;
  let interval;
  const window = { location: { reload() { reloads++; } } };
  window.parent = iframe ? {} : window;
  vm.runInNewContext(source, {
    window,
    document: {
      querySelector(selector) {
        if (selector === 'meta[name="adocweave-preview-generation"]') return { content: String(generation) };
        assert.equal(selector, ".slides-diagnostics");
        return display;
      },
    },
    fetch: async (url, options) => {
      requests.push(url);
      assert.equal(options.cache, "no-store");
      return { json: async () => fetch() };
    },
    setInterval(callback, milliseconds) {
      assert.equal(milliseconds, 500);
      interval = callback;
    },
  });
  return { requests, display, get reloads() { return reloads; }, poll() { return interval(); }, get scheduled() { return Boolean(interval); } };
}

test("the first poll reloads when a rebuild completed after HTML was served", async () => {
  const view = client({ generation: 3, fetch: () => ({ generation: 4, diagnostics: [] }) });
  await settle();
  assert.equal(view.reloads, 1);
  assert.deepEqual(view.requests, ["/events"]);
});

test("diagnostics come from the same event and require no second request", async () => {
  const view = client({ generation: 4, fetch: () => ({ generation: 4, diagnostics: [
    { severity: "error", code: "slides-invalid-step", message: "invalid step" },
    { severity: "warning", code: "warning", message: "not an error" },
    { code: "preview-build", message: "missing file" },
  ] }) });
  await settle();
  assert.equal(view.reloads, 0);
  assert.deepEqual(view.requests, ["/events"]);
  assert.equal(view.display.textContent, "slides-invalid-step: invalid step\npreview-build: missing file");
  assert.equal(view.display.hidden, false);
});

test("a later generation reloads a page that initially matched", async () => {
  let event = { generation: 4, diagnostics: [] };
  const view = client({ generation: 4, fetch: () => event });
  await settle();
  assert.equal(view.reloads, 0);
  assert.equal(view.display.hidden, true);
  event = { generation: 5, diagnostics: [] };
  await view.poll();
  assert.equal(view.reloads, 1);
});

test("a pending poll is not duplicated and a failed request can be retried", async () => {
  let reject;
  let result = new Promise((_, rejectPromise) => { reject = rejectPromise; });
  const view = client({ fetch: () => result });
  await settle();
  await view.poll();
  assert.equal(view.requests.length, 1);
  reject(new Error("disconnected"));
  await settle();
  result = { generation: 1, diagnostics: [] };
  await view.poll();
  assert.equal(view.requests.length, 2);
  assert.equal(view.reloads, 0);
});

test("speaker iframes do not poll or initiate reloads", () => {
  const view = client({ iframe: true, fetch: () => assert.fail("unexpected request") });
  assert.equal(view.scheduled, false);
  assert.deepEqual(view.requests, []);
});
