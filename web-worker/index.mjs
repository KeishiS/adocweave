export {
  AdocWeaveClient,
  AdocWeaveError,
  isAdocWeaveLifecycleError,
} from "./client.mjs";
export { WASM_PACKAGE_VERSION } from "./contracts.mjs";
export { PROTOCOL_SCHEMA_VERSION } from "./worker-protocol.mjs";

export function defaultAssetUrls(baseUrl) {
  if (baseUrl === undefined) throw new TypeError("baseUrl is required");
  return {
    workerUrl: new URL("./worker.mjs", baseUrl),
    moduleUrl: new URL("../wasm/adocweave_wasm.js", baseUrl),
    wasmUrl: new URL("../wasm/adocweave_wasm_bg.wasm", baseUrl),
  };
}
