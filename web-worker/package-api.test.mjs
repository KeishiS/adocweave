import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";

import {
  AdocWeaveClient,
  AdocWeaveError,
  WASM_PACKAGE_VERSION,
  PROTOCOL_SCHEMA_VERSION,
  defaultAssetUrls,
  isAdocWeaveLifecycleError,
} from "./index.mjs";
import { WORKER_PROTOCOL_VERSION } from "./worker-protocol.mjs";

class FakeWorker {
  static created = [];
  listeners = new Map();
  removed = [];
  messages = [];
  terminated = false;
  autoReady = true;

  constructor() { FakeWorker.created.push(this); }
  addEventListener(type, listener) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set());
    this.listeners.get(type).add(listener);
  }
  removeEventListener(type, listener) {
    this.removed.push([type, listener]);
    this.listeners.get(type)?.delete(listener);
  }
  postMessage(message) {
    this.messages.push(message);
    if (message.type === "init" && this.autoReady) {
      queueMicrotask(() => this.publish({
        protocolVersion: WORKER_PROTOCOL_VERSION,
        type: "ready",
      }));
    }
  }
  terminate() { this.terminated = true; }
  publish(data) {
    for (const listener of this.listeners.get("message") ?? []) listener({ data });
  }
  fail(message = "worker failed") {
    for (const listener of this.listeners.get("error") ?? []) listener({ message });
  }
  messageFail() {
    for (const listener of this.listeners.get("messageerror") ?? []) listener({});
  }
}

function client(Worker = FakeWorker) {
  return new AdocWeaveClient({
    workerUrl: "worker.mjs", moduleUrl: "wasm.js", wasmUrl: "wasm.wasm", Worker,
  });
}

function result(requestId, value = { html: "ok" }) {
  return { type: "result", requestId, result: value };
}

function error(requestId, code = "invalid-request") {
  return {
    type: "error",
    requestId,
    error: { code, message: code },
  };
}

async function dispatched(worker) {
  await new Promise((resolve) => setTimeout(resolve, 0));
  return worker.messages.findLast((message) => message.type === "analyze");
}

test.beforeEach(() => { FakeWorker.created = []; });

test("public entry resolves assets without eager work", () => {
  assert.throws(() => defaultAssetUrls(), /baseUrl is required/);
  for (const base of [
    "https://example.test/pkg/worker/index.mjs",
    "https://example.test/assets/adocweave/worker/index.mjs?hash=vite",
    "https://cdn.example.test/webpack/adocweave/worker/index.mjs",
  ]) {
    const urls = defaultAssetUrls(base);
    const root = new URL("../", base);
    assert.equal(urls.workerUrl.href, new URL("worker/worker.mjs", root).href);
    assert.equal(urls.moduleUrl.href, new URL("wasm/adocweave_wasm.js", root).href);
    assert.equal(urls.wasmUrl.href, new URL("wasm/adocweave_wasm_bg.wasm", root).href);
  }
  assert.equal(FakeWorker.created.length, 0);
  assert.match(WASM_PACKAGE_VERSION, /^\d+\.\d+\.\d+(?:-rc\.[1-9]\d*)?$/);
  assert.ok(Number.isSafeInteger(PROTOCOL_SCHEMA_VERSION));
});

test("client exposes only analyze and dispose operations", async () => {
  assert.deepEqual(Object.getOwnPropertyNames(AdocWeaveClient.prototype).sort(), [
    "analyze", "constructor", "dispose",
  ]);
  const exports = await import("./index.mjs");
  for (const removed of [
    "AdocWeaveClientError", "AdocWeaveResult", "AdocWeaveWorkerClient", "analyzeOnce",
    "PACKAGE_VERSION",
  ]) {
    assert.equal(Object.hasOwn(exports, removed), false, removed);
  }
});

test("analyze sends the public request unchanged with one requestId", async () => {
  const instance = client();
  const request = {
    source: { text: "include::part.adoc[]" },
    products: { html: true },
    resources: { documents: { "part.adoc": "text" } },
  };
  const analysis = instance.analyze(request);
  const worker = FakeWorker.created[0];
  const message = await dispatched(worker);
  assert.deepEqual(Object.keys(worker.messages[0]).sort(), [
    "moduleUrl", "protocolVersion", "type", "wasmUrl",
  ]);
  assert.deepEqual(Object.keys(message).sort(), [
    "payload", "requestId", "type",
  ]);
  assert.equal(message.requestId, 1);
  assert.equal(message.payload, request);
  worker.publish(result(1, { html: "done" }));
  assert.deepEqual(await analysis, { html: "done" });
  instance.dispose();
});

test("a second analysis is rejected without disturbing the active request", async () => {
  const instance = client();
  const first = instance.analyze({ source: { text: "first" }, products: { html: true } });
  await assert.rejects(
    instance.analyze({ source: { text: "second" }, products: { html: true } }),
    errorWithCode("analysis-in-progress"),
  );
  const worker = FakeWorker.created[0];
  const message = await dispatched(worker);
  worker.publish(result(message.requestId));
  await first;
  assert.equal(worker.terminated, false);
  instance.dispose();
});

test("a pre-aborted signal rejects with the common cancelled error", async () => {
  const controller = new AbortController();
  controller.abort();
  const instance = client();
  await assert.rejects(
    instance.analyze({ source: { text: "text" }, products: { html: true } }, { signal: controller.signal }),
    errorWithCode("cancelled"),
  );
  assert.equal(FakeWorker.created.length, 0);
  instance.dispose();
});

test("Abort during initialization settles and permits a fresh worker", async () => {
  class SlowWorker extends FakeWorker { autoReady = false; }
  const controller = new AbortController();
  const instance = client(SlowWorker);
  const aborted = instance.analyze({ source: { text: "first" }, products: { html: true } }, { signal: controller.signal });
  const oldWorker = FakeWorker.created[0];
  controller.abort();
  await assert.rejects(aborted, errorWithCode("cancelled"));
  assert.equal(oldWorker.terminated, true);
  assert.equal(oldWorker.removed.length, 3);

  const next = instance.analyze({ source: { text: "next" }, products: { html: true } });
  const newWorker = FakeWorker.created[1];
  newWorker.publish({ protocolVersion: WORKER_PROTOCOL_VERSION, type: "ready" });
  const message = await dispatched(newWorker);
  newWorker.publish(result(message.requestId, { html: "fresh" }));
  assert.deepEqual(await next, { html: "fresh" });
  instance.dispose();
});

test("Abort during execution removes its listener and starts fresh next time", async () => {
  const controller = new AbortController();
  let added = 0;
  let removed = 0;
  const originalAdd = controller.signal.addEventListener.bind(controller.signal);
  const originalRemove = controller.signal.removeEventListener.bind(controller.signal);
  controller.signal.addEventListener = (...args) => { ++added; return originalAdd(...args); };
  controller.signal.removeEventListener = (...args) => { ++removed; return originalRemove(...args); };

  const instance = client();
  const analysis = instance.analyze({ source: { text: "first" }, products: { html: true } }, { signal: controller.signal });
  const oldWorker = FakeWorker.created[0];
  await dispatched(oldWorker);
  controller.abort();
  await assert.rejects(analysis, errorWithCode("cancelled"));
  assert.equal(oldWorker.terminated, true);
  assert.equal(added, 1);
  assert.equal(removed, 1);

  const next = instance.analyze({ source: { text: "next" }, products: { html: true } });
  const newWorker = FakeWorker.created[1];
  const message = await dispatched(newWorker);
  newWorker.publish(result(message.requestId));
  await next;
  instance.dispose();
});

test("ordinary WASM errors settle the Promise and reuse the worker", async () => {
  const instance = client();
  const first = instance.analyze({ source: { text: "bad" }, products: { html: true } });
  const worker = FakeWorker.created[0];
  const firstMessage = await dispatched(worker);
  worker.publish(error(firstMessage.requestId));
  await assert.rejects(first, errorWithCode("invalid-request"));
  assert.equal(worker.terminated, false);

  const second = instance.analyze({ source: { text: "good" }, products: { html: true } });
  const secondMessage = await dispatched(worker);
  assert.equal(secondMessage.requestId, firstMessage.requestId + 1);
  worker.publish(result(secondMessage.requestId));
  await second;
  assert.equal(FakeWorker.created.length, 1);
  instance.dispose();
});

test("initialization errors settle the Promise and start fresh next time", async () => {
  class SlowWorker extends FakeWorker { autoReady = false; }
  const instance = client(SlowWorker);
  const failed = instance.analyze({ source: { text: "first" }, products: { html: true } });
  const oldWorker = FakeWorker.created[0];
  oldWorker.publish({
    type: "initialization-error",
    error: { code: "worker-failed", message: "WASM initialization failed" },
  });
  await assert.rejects(failed, errorWithCode("worker-failed"));
  assert.equal(oldWorker.terminated, true);

  const next = instance.analyze({ source: { text: "next" }, products: { html: true } });
  const newWorker = FakeWorker.created[1];
  newWorker.publish({ protocolVersion: WORKER_PROTOCOL_VERSION, type: "ready" });
  const message = await dispatched(newWorker);
  newWorker.publish(result(message.requestId));
  await next;
  instance.dispose();
});

test("fatal responses discard the worker and the next request starts fresh", async () => {
  const instance = client();
  const first = instance.analyze({ source: { text: "trap" }, products: { html: true } });
  const oldWorker = FakeWorker.created[0];
  const message = await dispatched(oldWorker);
  oldWorker.publish({
    type: "fatal",
    requestId: message.requestId,
    error: { code: "wasm-trapped", message: "unreachable" },
  });
  await assert.rejects(first, errorWithCode("wasm-trapped"));
  assert.equal(oldWorker.terminated, true);

  const next = instance.analyze({ source: { text: "fresh" }, products: { html: true } });
  const newWorker = FakeWorker.created[1];
  const nextMessage = await dispatched(newWorker);
  newWorker.publish(result(nextMessage.requestId));
  await next;
  instance.dispose();
});

test("old worker events cannot settle a current request", async () => {
  const controller = new AbortController();
  const instance = client();
  const oldAnalysis = instance.analyze({ source: { text: "old" }, products: { html: true } }, { signal: controller.signal });
  const oldWorker = FakeWorker.created[0];
  const oldMessage = await dispatched(oldWorker);
  const staleListener = [...oldWorker.listeners.get("message")][0];
  controller.abort();
  await assert.rejects(oldAnalysis, errorWithCode("cancelled"));

  const current = instance.analyze({ source: { text: "current" }, products: { html: true } });
  const currentWorker = FakeWorker.created[1];
  const currentMessage = await dispatched(currentWorker);
  staleListener({ data: result(oldMessage.requestId, { html: "stale" }) });
  assert.equal(currentWorker.terminated, false);
  currentWorker.publish(result(currentMessage.requestId, { html: "current" }));
  assert.deepEqual(await current, { html: "current" });
  instance.dispose();
});

test("a mismatched requestId rejects and discards the current worker", async () => {
  const instance = client();
  const analysis = instance.analyze({ source: { text: "text" }, products: { html: true } });
  const worker = FakeWorker.created[0];
  const message = await dispatched(worker);
  worker.publish(result(message.requestId + 1));
  await assert.rejects(analysis, errorWithCode("worker-failed"));
  assert.equal(worker.terminated, true);
  instance.dispose();
});

test("an invalid or obsolete worker response settles and discards the worker", async () => {
  for (const response of [
    { type: "unknown", protocolVersion: WORKER_PROTOCOL_VERSION },
    { type: "ready", protocolVersion: WORKER_PROTOCOL_VERSION - 1 },
  ]) {
    const instance = client();
    const analysis = instance.analyze({ source: { text: "text" }, products: { html: true } });
    const worker = FakeWorker.created.at(-1);
    worker.publish(response);
    await assert.rejects(
      analysis,
      errorWithCode(response.type === "ready"
        ? "unsupported-worker-protocol"
        : "worker-failed"),
    );
    assert.equal(worker.terminated, true);
    assert.equal(worker.removed.length, 3);
    instance.dispose();
  }
});

test("initialization responses are rejected after analysis starts", async () => {
  for (const response of [
    { protocolVersion: WORKER_PROTOCOL_VERSION, type: "ready" },
    {
      type: "initialization-error",
      error: { code: "worker-failed", message: "late initialization failure" },
    },
  ]) {
    const instance = client();
    const analysis = instance.analyze({ source: { text: "text" }, products: { html: true } });
    const worker = FakeWorker.created.at(-1);
    await dispatched(worker);
    worker.publish(response);
    await assert.rejects(analysis, errorWithCode("worker-failed"));
    assert.equal(worker.terminated, true);
    instance.dispose();
  }
});

test("constructor, init postMessage, and worker errors settle the active Promise", async () => {
  const constructorFailure = client(class { constructor() { throw new Error("constructor failed"); } });
  await assert.rejects(constructorFailure.analyze({ source: { text: "text" }, products: { html: true } }), errorWithCode("worker-failed"));

  class PostMessageFailure extends FakeWorker { postMessage() { throw new Error("postMessage failed"); } }
  const postFailure = client(PostMessageFailure);
  await assert.rejects(postFailure.analyze({ source: { text: "text" }, products: { html: true } }), errorWithCode("worker-failed"));
  assert.equal(FakeWorker.created.at(-1).terminated, true);

  const workerFailure = client();
  const analysis = workerFailure.analyze({ source: { text: "text" }, products: { html: true } });
  const worker = FakeWorker.created.at(-1);
  await dispatched(worker);
  worker.fail();
  await assert.rejects(analysis, errorWithCode("worker-failed"));
  assert.equal(worker.terminated, true);
});

test("analyze postMessage and message decoding failures settle the active Promise", async () => {
  class AnalyzePostFailure extends FakeWorker {
    postMessage(message) {
      super.postMessage(message);
      if (message.type === "analyze") throw new Error("analyze postMessage failed");
    }
  }
  const postFailure = client(AnalyzePostFailure);
  await assert.rejects(postFailure.analyze({ source: { text: "text" }, products: { html: true } }), errorWithCode("worker-failed"));
  assert.equal(FakeWorker.created.at(-1).terminated, true);

  const decodingFailure = client();
  const analysis = decodingFailure.analyze({ source: { text: "text" }, products: { html: true } });
  const worker = FakeWorker.created.at(-1);
  await dispatched(worker);
  worker.messageFail();
  await assert.rejects(analysis, errorWithCode("worker-failed"));
  assert.equal(worker.terminated, true);
});

test("dispose settles the active Promise and removes all worker listeners", async () => {
  const instance = client();
  const analysis = instance.analyze({ source: { text: "text" }, products: { html: true } });
  const worker = FakeWorker.created[0];
  instance.dispose();
  await assert.rejects(analysis, errorWithCode("disposed"));
  assert.equal(worker.terminated, true);
  assert.equal(worker.removed.length, 3);
  await assert.rejects(instance.analyze({ source: { text: "again" }, products: { html: true } }), errorWithCode("disposed"));
});

test("SSR import and client construction remain lazy", async () => {
  const originalWorker = globalThis.Worker;
  try {
    globalThis.Worker = undefined;
    const imported = await import(`./index.mjs?ssr=${Date.now()}`);
    const instance = new imported.AdocWeaveClient({
      workerUrl: "worker.mjs", moduleUrl: "wasm.js", wasmUrl: "wasm.wasm",
    });
    await assert.rejects(instance.analyze({ source: { text: "text" }, products: { html: true } }), errorWithCode("worker-failed"));
  } finally {
    globalThis.Worker = originalWorker;
  }
});

test("public lifecycle union matches the runtime type guard", async () => {
  const source = await readFile(new URL("./client.mjs", import.meta.url), "utf8");
  const runtimeBlock = source.match(/const LIFECYCLE_ERROR_CODES = new Set\(\[([\s\S]*?)\]\)/);
  const types = await readFile(new URL("./index.d.mts", import.meta.url), "utf8");
  const typeBlock = types.match(/export type AdocWeaveLifecycleErrorCode =([\s\S]*?);/);
  assert.ok(runtimeBlock);
  assert.ok(typeBlock);
  const codes = (block) => [...block.matchAll(/"([a-z-]+)"/g)]
    .map((match) => match[1]).sort();
  assert.deepEqual(codes(runtimeBlock[1]), codes(typeBlock[1]));
  for (const code of codes(runtimeBlock[1])) {
    assert.equal(isAdocWeaveLifecycleError(
      new AdocWeaveError({ code, message: code }),
    ), true);
  }
});

function errorWithCode(code) {
  return (value) => value instanceof AdocWeaveError && value.code === code;
}
