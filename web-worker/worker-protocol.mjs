// Worker envelopeの定数と形の検査。
//
// 中身(WasmRequestとWasmResponse)の検査はWebAssembly側のserdeが行います。requestは
// 未知fieldを拒否して構造化errorを返し、responseは同じ境界が生成します。ここでは、
// workerとclientがやり取りする封筒の形だけを確かめます。

export const PROTOCOL_SCHEMA_VERSION = 14;
export const WORKER_PROTOCOL_VERSION = 2;
export const PACKAGE_VERSION = "0.46.1";

const string = (value) => typeof value === "string";
const u32 = (value) => Number.isInteger(value) && value >= 0 && value <= 4294967295;
const number = (value) => typeof value === "number" && Number.isFinite(value);
const object = (value) =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const nullableU32 = (value) => value === null || u32(value);
const cancellationBuffer = (value) =>
  value === null ||
  (typeof SharedArrayBuffer === "function" && value instanceof SharedArrayBuffer);

/// 各封筒が持つfieldと、その値の検査。
const ENVELOPES = {
  requests: {
    initialize: {
      protocolVersion: u32,
      moduleUrl: string,
      wasmUrl: string,
      debounceMs: number,
      cancellationBuffer,
    },
    analyze: {
      protocolVersion: u32,
      version: u32,
      generation: u32,
      payload: object,
    },
  },
  responses: {
    ready: { protocolVersion: u32 },
    result: {
      protocolVersion: u32,
      version: u32,
      generation: u32,
      result: object,
    },
    error: {
      protocolVersion: u32,
      version: u32,
      generation: u32,
      error: (value) => object(value) && string(value.code) && string(value.message),
    },
  },
};

const CLIENT_ERROR = {
  code: string,
  message: string,
  sourceVersion: nullableU32,
  generation: u32,
};

/// 封筒ごとのfield名。testが送信messageの形を照合するために使います。
export const WORKER_MESSAGE_FIELDS = Object.fromEntries(
  Object.entries(ENVELOPES).flatMap(([direction, variants]) =>
    Object.entries(variants).map(([variant, fields]) => [
      `${direction}.${variant}`,
      ["type", ...Object.keys(fields)],
    ]),
  ),
);

function matches(value, fields) {
  if (!object(value)) return false;
  const names = Object.keys(fields);
  if (Object.keys(value).some((name) => name !== "type" && !names.includes(name))) return false;
  return names.every((name) => Object.hasOwn(value, name) && fields[name](value[name]));
}

export function validateWorkerMessage(value, direction) {
  const variants = ENVELOPES[direction];
  if (variants === undefined || !object(value) || typeof value.type !== "string") return false;
  const fields = variants[value.type];
  return fields !== undefined && matches(value, fields);
}

export function validateClientError(value) {
  return matches(value, CLIENT_ERROR);
}
