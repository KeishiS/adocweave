// WorkerとNodeの入口は、同じ要求objectを変更せずWebAssemblyへ渡します。

import { PROCESSING_ERROR_CODES } from "./worker-protocol.mjs";

export function analysisPayload(request) {
  try {
    globalThis.structuredClone?.(request);
  } catch {
    throw {
      code: "invalid-request",
      message: "the analysis request must be structured-cloneable",
    };
  }
  return request;
}

export function parseWasmError(cause) {
  return typeof cause === "object" && cause !== null &&
      PROCESSING_ERROR_CODES.has(cause.code) && typeof cause.message === "string"
    ? { code: cause.code, message: cause.message }
    : null;
}
