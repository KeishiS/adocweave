// WorkerとNodeの入口が同じ要求を組み立て、同じ方法でWASMの構造化errorを読むための
// 共有部分。片方だけが要求の既定値を変えると、同じ入力から違う結果が出てしまう。

export function analysisPayload({
  sourceId = null,
  source,
  preprocess,
  products,
  renderInputs,
  analysisOptions = {},
  renderPolicy = {},
  outputLimits = {},
}) {
  const payload = { sourceId, source, analysisOptions, renderPolicy, outputLimits };
  if (products !== undefined) payload.products = products;
  if (preprocess !== undefined) payload.preprocess = preprocess;
  if (renderInputs !== undefined) payload.renderInputs = renderInputs;
  return payload;
}

// WASMは扱えた失敗をJSON文字列としてthrowする。それ以外はtrapとして扱う。
export function parseWasmError(cause) {
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
