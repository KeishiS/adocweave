import type { WasmRequest, WasmResponse } from "./protocol.d.mts";

export type * from "./protocol.d.mts";
export declare const PROTOCOL_SCHEMA_VERSION: number;

export type AdocWeaveResult = WasmResponse;

type Settings<T> = {
  [K in keyof T]?: T[K] extends ReadonlyArray<unknown>
    ? T[K]
    : T[K] extends object
      ? Settings<T[K]>
      : T[K];
};

export type AnalyzeRequest = Pick<WasmRequest, "source"> & {
  sourceId?: WasmRequest["sourceId"];
  preprocess?: WasmRequest["preprocess"];
} & Settings<
    Pick<
      WasmRequest,
      "products" | "renderInputs" | "analysisOptions" | "renderPolicy" | "outputLimits"
    >
  >;

export interface AdocWeaveClientOptions {
  workerUrl: string | URL;
  moduleUrl: string | URL;
  wasmUrl: string | URL;
  Worker?: typeof Worker;
}

export type AdocWeaveClientLifecycleErrorCode =
  | "analysis-in-progress"
  | "disposed"
  | "invalid-worker-response"
  | "unsupported-worker-protocol"
  | "wasm-trapped"
  | "worker-failed";

export declare class AdocWeaveClientError<Code extends string = string> extends Error {
  constructor(error: { code: Code; message: string });
  readonly code: Code;
}

export declare function isAdocWeaveClientLifecycleError(
  error: unknown,
): error is AdocWeaveClientError<AdocWeaveClientLifecycleErrorCode>;

export declare class AdocWeaveClient {
  constructor(options: AdocWeaveClientOptions);
  analyze(
    request: AnalyzeRequest,
    options?: { signal?: AbortSignal },
  ): Promise<AdocWeaveResult>;
  dispose(): void;
}

export declare function defaultAssetUrls(baseUrl?: string | URL): {
  workerUrl: URL;
  moduleUrl: URL;
  wasmUrl: URL;
};
export declare const BROWSER_PACKAGE_VERSION: string;
