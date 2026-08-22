export {
  AdocWeaveClient,
  AdocWeaveClientError,
  isAdocWeaveClientLifecycleError,
} from "./client.mjs";
export { BROWSER_PACKAGE_VERSION } from "./contracts.mjs";
export { PROTOCOL_SCHEMA_VERSION } from "./worker-protocol.mjs";

export function defaultAssetUrls(baseUrl = import.meta.url) {
  return {
    workerUrl: new URL("./worker.mjs", baseUrl),
    moduleUrl: new URL("../wasm/adocweave_wasm.js", baseUrl),
    wasmUrl: new URL("../wasm/adocweave_wasm_bg.wasm", baseUrl),
  };
}
