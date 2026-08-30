// WorkerとNodeの入口は、同じ上限内で固定したdata propertyだけの要求objectを渡します。

import { PROCESSING_ERROR_CODES } from "./worker-protocol.mjs";

const MAX_DEPTH = 128;
const MAX_ARRAY_LENGTH = 20_000;
const MAX_OBJECT_KEYS = 20_000;
const MAX_TOTAL_NODES = 100_000;
const MAX_TOTAL_KEYS = 100_000;
const MAX_PROPERTY_NAME_UTF16_UNITS = 1_024;
const MAX_STRING_UTF16_UNITS = 16 * 1024 * 1024;
const MAX_TOTAL_STRING_UTF16_UNITS = 32 * 1024 * 1024;
const MAX_TOTAL_STRING_UTF8_BYTES = 32 * 1024 * 1024;

class SnapshotLimitError extends Error {
  constructor(resource) {
    super(`${resource} exceeds the fixed WebAssembly boundary limit`);
  }
}

export function analysisPayload(request) {
  try {
    return snapshot(request, new WeakSet(), 0, {
      nodes: 0,
      keys: 0,
      stringUtf16Units: 0,
      stringUtf8Bytes: 0,
    });
  } catch (cause) {
    if (cause instanceof SnapshotLimitError) {
      throw {
        code: "input-limit-exceeded",
        message: cause.message,
      };
    }
    throw {
      code: "invalid-request",
      message: "the analysis request must be structured-cloneable",
    };
  }
}

function snapshot(value, ancestors, depth, budget) {
  if (depth >= MAX_DEPTH) limit("request nesting depth");
  budget.nodes += 1;
  if (budget.nodes > MAX_TOTAL_NODES) limit("request node count");

  if (typeof value === "string") {
    inspectString(value, MAX_STRING_UTF16_UNITS, budget);
    return value;
  }
  if (
    value === null || value === undefined || typeof value === "boolean" ||
    typeof value === "number" || typeof value === "bigint"
  ) {
    return value;
  }
  if (typeof value !== "object") invalid();
  if (ancestors.has(value)) invalid();

  const array = Array.isArray(value);
  const prototype = Object.getPrototypeOf(value);
  if (
    (array && prototype !== Array.prototype) ||
    (!array && prototype !== Object.prototype && prototype !== null)
  ) {
    invalid();
  }

  ancestors.add(value);
  try {
    return array
      ? snapshotArray(value, ancestors, depth, budget)
      : snapshotObject(value, ancestors, depth, budget);
  } finally {
    ancestors.delete(value);
  }
}

function snapshotArray(value, ancestors, depth, budget) {
  const keys = ownStringKeys(value, budget);
  const lengthDescriptor = dataPropertyDescriptor(value, "length", false);
  const length = lengthDescriptor.value;
  if (
    typeof length !== "number" || !Number.isFinite(length) || length < 0 ||
    !Number.isInteger(length)
  ) {
    invalid();
  }
  if (length > MAX_ARRAY_LENGTH) limit("request array length");

  const result = new Array(length);
  for (const key of keys) {
    if (key === "length") continue;
    const index = Number(key);
    if (!Number.isInteger(index) || index < 0 || String(index) !== key || index >= length) {
      invalid();
    }
    const descriptor = dataPropertyDescriptor(value, key, true);
    const field = snapshot(descriptor.value, ancestors, depth + 1, budget);
    defineDataProperty(result, key, field);
  }
  return result;
}

function snapshotObject(value, ancestors, depth, budget) {
  const keys = ownStringKeys(value, budget);
  const result = {};
  for (const key of keys) {
    const descriptor = dataPropertyDescriptor(value, key, true);
    const field = snapshot(descriptor.value, ancestors, depth + 1, budget);
    defineDataProperty(result, key, field);
  }
  return result;
}

function ownStringKeys(value, budget) {
  // ECMAScriptには遅延してown keyを得るAPIがないため、Reflect.ownKeysが作る
  // key配列だけは事前に全件生成されます。以後のdescriptor走査とsnapshot確保は
  // この件数上限で停止します。
  const keys = Reflect.ownKeys(value);
  if (keys.length > MAX_OBJECT_KEYS) limit("request object key count");
  budget.keys += keys.length;
  if (budget.keys > MAX_TOTAL_KEYS) limit("request object key count");
  for (const key of keys) {
    if (typeof key !== "string") invalid();
    inspectString(key, MAX_PROPERTY_NAME_UTF16_UNITS, budget);
  }
  return keys;
}

function dataPropertyDescriptor(value, key, requireEnumerable) {
  const descriptor = Reflect.getOwnPropertyDescriptor(value, key);
  if (descriptor === undefined || !Object.hasOwn(descriptor, "value")) invalid();
  if (requireEnumerable && descriptor.enumerable !== true) invalid();
  return descriptor;
}

function defineDataProperty(target, key, value) {
  const descriptor = Object.create(null);
  descriptor.value = value;
  descriptor.writable = true;
  descriptor.enumerable = true;
  descriptor.configurable = true;
  if (!Reflect.defineProperty(target, key, descriptor)) {
    invalid();
  }
}

function inspectString(value, maximumUtf16Units, budget) {
  const utf16Units = value.length;
  if (utf16Units > maximumUtf16Units) limit("request string length");
  budget.stringUtf16Units += utf16Units;
  if (budget.stringUtf16Units > MAX_TOTAL_STRING_UTF16_UNITS) {
    limit("request string length");
  }
  budget.stringUtf8Bytes += utf8Length(value);
  if (budget.stringUtf8Bytes > MAX_TOTAL_STRING_UTF8_BYTES) {
    limit("request string bytes");
  }
}

function utf8Length(value) {
  let bytes = 0;
  for (let index = 0; index < value.length; index += 1) {
    const unit = value.charCodeAt(index);
    if (unit <= 0x7f) {
      bytes += 1;
    } else if (unit <= 0x7ff) {
      bytes += 2;
    } else if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        bytes += 4;
        index += 1;
      } else {
        bytes += 3;
      }
    } else {
      bytes += 3;
    }
  }
  return bytes;
}

function limit(resource) {
  throw new SnapshotLimitError(resource);
}

function invalid() {
  throw new TypeError("invalid analysis request snapshot");
}

export function parseWasmError(cause) {
  return typeof cause === "object" && cause !== null &&
      PROCESSING_ERROR_CODES.has(cause.code) && typeof cause.message === "string"
    ? { code: cause.code, message: cause.message }
    : null;
}
