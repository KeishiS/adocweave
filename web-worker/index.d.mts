import type { WasmRequest, WasmResponse } from "./protocol.d.mts";
import type { AdocWeaveError } from "./worker-protocol.d.mts";

export type * from "./protocol.d.mts";
export type { AdocWeaveError, WorkerRequest, WorkerResponse } from "./worker-protocol.d.mts";
export { PROTOCOL_SCHEMA_VERSION } from "./worker-protocol.mjs";

/// 利用側が使う名前。wire型はRustの定義から生成するため、公開名だけここで与えます。
export type UpdateRequest = WasmRequest;
export type AdocWeaveWasmResponse = WasmResponse;

export type AdocWeaveResult =
  Omit<WasmResponse, "version"> & { sourceVersion: number };

export interface AdocWeaveClientOptions {
  workerUrl: string | URL;
  moduleUrl: string | URL;
  wasmUrl: string | URL;
  debounceMs?: number;
  onResult?: (result: AdocWeaveResult) => void;
  onError?: (error: AdocWeaveError) => void;
  Worker?: typeof Worker;
  sharedCancellation?: boolean;
}

export type AdocWeaveClientLifecycleErrorCode =
  | "cancelled"
  | "disposed"
  | "invalid-worker-response"
  | "superseded"
  | "unsupported-package-version"
  | "unsupported-worker-protocol"
  | "wasm-trapped"
  | "worker-failed";

export declare class AdocWeaveClientError<Code extends string = string> extends Error {
  constructor(error: {
    code: Code;
    message: string;
    sourceVersion: number | null;
    generation: number;
  });
  readonly code: Code;
  readonly sourceVersion: number | null;
  readonly generation: number;
}

export declare function isAdocWeaveClientLifecycleError(
  error: unknown,
): error is AdocWeaveClientError<AdocWeaveClientLifecycleErrorCode>;

export declare class AdocWeaveClient {
  constructor(options: AdocWeaveClientOptions);
  readonly ready: Promise<void>;
  analyze(request: UpdateRequest): Promise<AdocWeaveResult>;
  update(request: UpdateRequest): number;
  cancel(): void;
  dispose(): void;
}

export { AdocWeaveClient as AdocWeaveWorkerClient };
export declare function defaultAssetUrls(baseUrl?: string | URL): {
  workerUrl: URL;
  moduleUrl: URL;
  wasmUrl: URL;
};
export declare function analyzeOnce(
  clientOptions: AdocWeaveClientOptions,
  request: UpdateRequest,
): Promise<AdocWeaveResult>;
export declare const BROWSER_PACKAGE_VERSION: string;
export declare const PACKAGE_VERSION: string;
