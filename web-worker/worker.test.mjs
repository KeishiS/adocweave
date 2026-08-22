import assert from "node:assert/strict";
import test from "node:test";

import {
  PROTOCOL_SCHEMA_VERSION,
  WORKER_MESSAGE_FIELDS,
  WORKER_PROTOCOL_VERSION,
  validateWorkerMessage,
} from "./worker-protocol.mjs";

function moduleUrl(processBody) {
  const source = `
    export default async function init() {}
    export function protocolSchemaVersion() { return ${PROTOCOL_SCHEMA_VERSION}; }
    export function process(request) { ${processBody} }
  `;
  return `data:text/javascript,${encodeURIComponent(source)}`;
}

async function harness(processBody) {
  const previousSelf = globalThis.self;
  const messages = [];
  let closes = 0;
  globalThis.self = {
    postMessage: (message) => messages.push(message),
    close: () => { ++closes; },
  };
  await import(`./worker.mjs?worker-test=${Date.now()}-${Math.random()}`);
  await globalThis.self.onmessage({
    data: {
      protocolVersion: WORKER_PROTOCOL_VERSION,
      type: "init",
      moduleUrl: moduleUrl(processBody),
      wasmUrl: "unused.wasm",
    },
  });
  return {
    messages,
    get closes() { return closes; },
    async analyze(requestId, payload = { source: "text" }) {
      await globalThis.self.onmessage({
        data: {
          type: "analyze",
          requestId,
          payload,
        },
      });
    },
    restore() { globalThis.self = previousSelf; },
  };
}

test("worker envelope has one requestId and exact fields", () => {
  const samples = {
    "requests.init": {
      type: "init", protocolVersion: WORKER_PROTOCOL_VERSION,
      moduleUrl: "module.js", wasmUrl: "module.wasm",
    },
    "requests.analyze": {
      type: "analyze", requestId: 1, payload: {},
    },
    "responses.ready": { type: "ready", protocolVersion: WORKER_PROTOCOL_VERSION },
    "responses.result": {
      type: "result", requestId: 1, result: {},
    },
    "responses.error": {
      type: "error", requestId: 1, error: { code: "invalid-request", message: "invalid" },
    },
    "responses.fatal": {
      type: "fatal", requestId: 1, error: { code: "worker-failed", message: "failed" },
    },
  };
  for (const [contract, sample] of Object.entries(samples)) {
    const direction = contract.startsWith("requests") ? "requests" : "responses";
    assert.deepEqual(Object.keys(sample).sort(), [...WORKER_MESSAGE_FIELDS[contract]].sort());
    assert.equal(validateWorkerMessage(sample, direction), true, contract);
    assert.equal(validateWorkerMessage({ ...sample, unexpected: true }, direction), false, contract);
    for (const field of Object.keys(sample)) {
      const missing = { ...sample };
      delete missing[field];
      assert.equal(validateWorkerMessage(missing, direction), false, `${contract}.${field}`);
    }
  }
});

test("worker initializes WASM and returns its result unchanged", async () => {
  const state = await harness("return { html: request.source };");
  try {
    assert.deepEqual(state.messages, [{
      protocolVersion: WORKER_PROTOCOL_VERSION,
      type: "ready",
    }]);
    await state.analyze(7, { source: "result" });
    assert.deepEqual(state.messages[1], {
      type: "result",
      requestId: 7,
      result: { html: "result" },
    });
    assert.equal(state.closes, 0);
  } finally { state.restore(); }
});

test("ordinary structured WASM errors do not close the worker", async () => {
  const state = await harness(`
    if (request.source === "bad") {
      throw JSON.stringify({ code: "invalid-request", message: "bad input" });
    }
    return { html: request.source };
  `);
  try {
    await state.analyze(1, { source: "bad" });
    assert.deepEqual(state.messages[1], {
      type: "error",
      requestId: 1,
      error: { code: "invalid-request", message: "bad input" },
    });
    assert.equal(state.closes, 0);
    await state.analyze(2, { source: "good" });
    assert.equal(state.messages[2].type, "result");
    assert.equal(state.messages[2].requestId, 2);
  } finally { state.restore(); }
});

test("a WebAssembly trap publishes fatal and closes the worker", async () => {
  const state = await harness('throw new WebAssembly.RuntimeError("unreachable");');
  try {
    await state.analyze(3);
    assert.deepEqual(state.messages[1], {
      type: "fatal",
      requestId: 3,
      error: { code: "wasm-trapped", message: "unreachable" },
    });
    assert.equal(state.closes, 1);
    await state.analyze(4);
    assert.equal(state.messages.length, 2);
  } finally { state.restore(); }
});

test("an unexpected exception publishes fatal and closes the worker", async () => {
  const state = await harness('throw new Error("unexpected");');
  try {
    await state.analyze(9);
    assert.equal(state.messages[1].type, "fatal");
    assert.equal(state.messages[1].requestId, 9);
    assert.deepEqual(state.messages[1].error, { code: "worker-failed", message: "unexpected" });
    assert.equal(state.closes, 1);
  } finally { state.restore(); }
});
