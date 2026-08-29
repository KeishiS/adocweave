// Workerの封筒では、解析要求を内部requestId一つだけで識別します。
// payloadとresultのschemaはWebAssembly境界で検査します。

export const PROTOCOL_SCHEMA_VERSION = 16;
export const WORKER_PROTOCOL_VERSION = 3;

const string = (value) => typeof value === "string";
const u32 = (value) => Number.isInteger(value) && value >= 0 && value <= 4294967295;
const object = (value) =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const error = (value) => object(value) && string(value.code) && string(value.message);

const ENVELOPES = {
  requests: {
    init: {
      protocolVersion: u32,
      moduleUrl: string,
      wasmUrl: string,
    },
    analyze: {
      requestId: u32,
      payload: object,
    },
  },
  responses: {
    ready: { protocolVersion: u32 },
    "initialization-error": { error },
    result: {
      requestId: u32,
      result: object,
    },
    error: {
      requestId: u32,
      error,
    },
    fatal: {
      requestId: u32,
      error,
    },
  },
};

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
