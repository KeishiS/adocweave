import type { AnalyzeRequest, AnalyzeResult } from "./protocol.d.mts";

export type * from "./protocol.d.mts";
export declare const PROTOCOL_SCHEMA_VERSION: number;

export interface AdocWeaveClientOptions {
  workerUrl: string | URL;
  moduleUrl: string | URL;
  wasmUrl: string | URL;
  Worker?: typeof Worker;
}

export type AdocWeaveErrorCode =
  | "invalid-request"
  | "input-limit-exceeded"
  | "output-limit-exceeded"
  | "analysis-failed"
  | "cancelled"
  | "analysis-in-progress"
  | "disposed"
  | "unsupported-worker-protocol"
  | "wasm-trapped"
  | "worker-failed";

export type AdocWeaveLifecycleErrorCode =
  | "cancelled"
  | "analysis-in-progress"
  | "disposed"
  | "unsupported-worker-protocol"
  | "wasm-trapped"
  | "worker-failed";

export declare class AdocWeaveError<Code extends AdocWeaveErrorCode = AdocWeaveErrorCode>
  extends Error {
  constructor(error: { code: Code; message: string });
  readonly code: Code;
}

export declare function isAdocWeaveLifecycleError(
  error: unknown,
): error is AdocWeaveError<AdocWeaveLifecycleErrorCode>;

export declare class AdocWeaveClient {
  constructor(options: AdocWeaveClientOptions);
  analyze(
    request: AnalyzeRequest,
    options?: { signal?: AbortSignal },
  ): Promise<AnalyzeResult>;
  dispose(): void;
}

export declare function defaultAssetUrls(baseUrl: string | URL): {
  workerUrl: URL;
  moduleUrl: URL;
  wasmUrl: URL;
};
