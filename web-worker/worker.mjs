import { parseWasmError } from "./analysis.mjs";
import {
  PROTOCOL_SCHEMA_VERSION,
  WORKER_PROTOCOL_VERSION,
  validateWorkerMessage,
} from "./worker-protocol.mjs";

let process = null;
let state = "new";

self.onmessage = async ({ data }) => {
  if (state === "closed") return;
  if (!validateWorkerMessage(data, "requests")) {
    const error = { code: "worker-failed", message: "invalid AdocWeave worker request" };
    if (
      state === "ready" && Number.isInteger(data?.requestId) &&
      data.requestId >= 0 && data.requestId <= 0xffff_ffff
    ) {
      fatal(data.requestId, error);
    } else {
      failInitialization(error);
    }
    return;
  }
  if (data.type === "init") {
    if (data.protocolVersion !== WORKER_PROTOCOL_VERSION) {
      failInitialization({
        code: "unsupported-worker-protocol",
        message: `expected worker protocol ${WORKER_PROTOCOL_VERSION}`,
      });
      return;
    }
    if (state !== "new") {
      failInitialization(new Error("invalid AdocWeave worker initialization"));
      return;
    }
    state = "initializing";
    try {
      const wasm = await import(data.moduleUrl);
      await wasm.default(data.wasmUrl);
      if (wasm.protocolSchemaVersion?.() !== PROTOCOL_SCHEMA_VERSION) {
        throw new Error("incompatible AdocWeave WASM protocol schema");
      }
      if (typeof wasm.process !== "function") {
        throw new Error("AdocWeave WASM process export is missing");
      }
      if (state !== "initializing") return;
      process = wasm.process;
      state = "ready";
      publish({
        protocolVersion: WORKER_PROTOCOL_VERSION,
        type: "ready",
      });
    } catch (cause) {
      failInitialization(cause);
    }
    return;
  }

  const { requestId } = data;
  if (state !== "ready" || process === null) {
    fatal(requestId, {
      code: "worker-failed",
      message: "AdocWeave worker was not initialized",
    });
    return;
  }

  try {
    const result = process(data.payload);
    publish({
      type: "result",
      requestId,
      result,
    });
  } catch (cause) {
    const wasmError = parseWasmError(cause);
    if (wasmError !== null) {
      publish({
        type: "error",
        requestId,
        error: wasmError,
      });
      return;
    }
    fatal(requestId, normalizeFatal(cause));
  }
};

function fatal(requestId, error) {
  state = "closed";
  try {
    publish({
      type: "fatal",
      requestId,
      error,
    });
  } finally {
    self.close();
  }
}

function failInitialization(cause) {
  if (state === "closed") return;
  state = "closed";
  try {
    publish({
      type: "initialization-error",
      error: normalizeFatal(cause),
    });
  } finally {
    self.close();
  }
}

function publish(message) {
  if (!validateWorkerMessage(message, "responses")) {
    throw new Error("invalid AdocWeave worker response");
  }
  self.postMessage(message);
}


function normalizeFatal(cause) {
  if (
    typeof cause === "object" && cause !== null && !(cause instanceof Error) &&
    typeof cause.code === "string" && typeof cause.message === "string"
  ) {
    return { code: cause.code, message: cause.message };
  }
  const message = cause instanceof Error ? cause.message : String(cause);
  if (
    typeof WebAssembly !== "undefined" &&
    typeof WebAssembly.RuntimeError === "function" &&
    cause instanceof WebAssembly.RuntimeError
  ) {
    return { code: "wasm-trapped", message };
  }
  return { code: "worker-failed", message };
}
