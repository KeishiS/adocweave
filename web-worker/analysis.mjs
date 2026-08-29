// WorkerとNodeの入口は、同じdata propertyだけの要求objectをWebAssemblyへ渡します。

import { PROCESSING_ERROR_CODES } from "./worker-protocol.mjs";

export function analysisPayload(request) {
  try {
    validateDataProperties(request, new WeakSet());
    globalThis.structuredClone?.(request);
  } catch {
    throw {
      code: "invalid-request",
      message: "the analysis request must be structured-cloneable",
    };
  }
  return request;
}

function validateDataProperties(value, seen) {
  if (value === null || typeof value !== "object" || seen.has(value)) return;
  seen.add(value);
  const array = Array.isArray(value);
  const prototype = Object.getPrototypeOf(value);
  if (
    (array && prototype !== Array.prototype) ||
    (!array && prototype !== Object.prototype && prototype !== null)
  ) {
    throw new TypeError("non-plain object");
  }
  for (const key of Reflect.ownKeys(value)) {
    if (typeof key === "symbol") throw new TypeError("symbol property");
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (descriptor === undefined || !Object.hasOwn(descriptor, "value")) {
      throw new TypeError("accessor property");
    }
    if (!descriptor.enumerable && !(array && key === "length")) {
      throw new TypeError("non-enumerable property");
    }
    validateDataProperties(descriptor.value, seen);
  }
}

export function parseWasmError(cause) {
  return typeof cause === "object" && cause !== null &&
      PROCESSING_ERROR_CODES.has(cause.code) && typeof cause.message === "string"
    ? { code: cause.code, message: cause.message }
    : null;
}
