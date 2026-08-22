import {
  PROTOCOL_SCHEMA_VERSION,
  WORKER_PROTOCOL_VERSION,
  validateWorkerMessage,
} from "./worker-protocol.mjs";

let process = null;
let closed = false;

self.onmessage = async ({ data }) => {
  if (closed) return;
  if (!validateWorkerMessage(data, "requests")) {
    throw new Error("invalid AdocWeave worker request");
  }
  if (data.type === "init") {
    if (data.protocolVersion !== WORKER_PROTOCOL_VERSION || process !== null) {
      throw new Error("invalid AdocWeave worker initialization");
    }
    const wasm = await import(data.moduleUrl);
    await wasm.default(data.wasmUrl);
    if (wasm.protocolSchemaVersion?.() !== PROTOCOL_SCHEMA_VERSION) {
      throw new Error("incompatible AdocWeave WASM protocol schema");
    }
    process = wasm.process;
    publish({
      protocolVersion: WORKER_PROTOCOL_VERSION,
      type: "ready",
    });
    return;
  }

  const { requestId } = data;
  if (process === null) {
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
  closed = true;
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

function publish(message) {
  if (!validateWorkerMessage(message, "responses")) {
    throw new Error("invalid AdocWeave worker response");
  }
  self.postMessage(message);
}

function parseWasmError(cause) {
  if (typeof cause !== "string") return null;
  try {
    const value = JSON.parse(cause);
    return typeof value === "object" && value !== null &&
      typeof value.code === "string" && typeof value.message === "string"
      ? { code: value.code, message: value.message }
      : null;
  } catch {
    return null;
  }
}

function normalizeFatal(cause) {
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
