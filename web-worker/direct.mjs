// ビルド時にNodeで解析と変換を行うための入口。Web Workerを使わず同じprocessで実行する。
// 取消しとWASM trapの分離は提供しない。単一の処理を順に実行する用途に限る。
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { analysisPayload, parseWasmError } from "./analysis.mjs";
import { AdocWeaveClientError } from "./client.mjs";
import { PROTOCOL_SCHEMA_VERSION } from "./worker-protocol.mjs";

export function defaultDirectAssetUrls(baseUrl = import.meta.url) {
  const base = new URL("./", baseUrl);
  return {
    moduleUrl: new URL("../wasm/adocweave_wasm.js", base),
    wasmUrl: new URL("../wasm/adocweave_wasm_bg.wasm", base),
  };
}

export async function createDirectAnalyzer(assets = defaultDirectAssetUrls()) {
  const {
    moduleUrl,
    wasmUrl,
    import: importModule = (href) => import(href),
    readWasm = (url) => readFileSync(fileURLToPath(url)),
  } = assets;
  let wasm;
  try {
    wasm = await importModule(new URL(moduleUrl).href);
    // web向けglueの既定初期化はfetchを使う。Nodeのfile URLでは読めないため、
    // byte列を自分で渡して同期的に初期化する。
    wasm.initSync({ module: readWasm(new URL(wasmUrl)) });
  } catch (cause) {
    throw new AdocWeaveClientError({
      code: "worker-failed",
      message: `AdocWeave WASM could not be initialized: ${cause?.message ?? cause}`,
    });
  }
  if (wasm.protocolSchemaVersion?.() !== PROTOCOL_SCHEMA_VERSION) {
    throw new AdocWeaveClientError({
      code: "unsupported-worker-protocol",
      message: `expected WASM protocol schema ${PROTOCOL_SCHEMA_VERSION}`,
    });
  }
  if (typeof wasm.process !== "function") {
    throw new AdocWeaveClientError({
      code: "worker-failed",
      message: "AdocWeave WASM process export is missing",
    });
  }
  return { analyze: (request) => runAnalysis(wasm.process, request) };
}

let shared = null;

export async function analyze(request) {
  shared ??= createDirectAnalyzer();
  const analyzer = await shared;
  return analyzer.analyze(request);
}

function runAnalysis(process, request) {
  try {
    return process(analysisPayload(request));
  } catch (cause) {
    const wasmError = parseWasmError(cause);
    if (wasmError !== null) throw new AdocWeaveClientError(wasmError);
    throw new AdocWeaveClientError({
      code: "wasm-trapped",
      message: `AdocWeave WASM trapped: ${cause?.message ?? cause}`,
    });
  }
}
