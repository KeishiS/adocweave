// WorkerとNodeの入口は、同じ要求objectを変更せずWebAssemblyへ渡します。

export function analysisPayload(request) {
  return request;
}

export function parseWasmError(cause) {
  return typeof cause === "object" && cause !== null &&
      typeof cause.code === "string" && typeof cause.message === "string"
    ? { code: cause.code, message: cause.message }
    : null;
}
