import type { WasmRequest, WasmResponse } from "./protocol.d.mts";
import type { AdocWeaveError } from "./worker-protocol.d.mts";

export type * from "./protocol.d.mts";
export type { AdocWeaveError, WorkerRequest, WorkerResponse } from "./worker-protocol.d.mts";
export { PROTOCOL_SCHEMA_VERSION } from "./worker-protocol.mjs";

/// 利用側が使う名前。wire型はRustの定義から生成するため、公開名だけここで与えます。
export type AdocWeaveWasmResponse = WasmResponse;

/// 設定objectのうち、省略できるfieldを任意にした形。
///
/// WebAssembly側は既定値を持つfieldの省略を受け付けます。生成したwire型は
/// 完全な形(すべてのfieldを持つ値)を表すため、入力として省略できる範囲は
/// ここで表現します。配列は要素ごとではなく丸ごと置き換えるため、そのままにします。
type Settings<T> = {
  [K in keyof T]?: T[K] extends ReadonlyArray<unknown>
    ? T[K]
    : T[K] extends object
      ? Settings<T[K]>
      : T[K];
};

/// 解析を依頼するときに渡す値。``packageVersion``と``generation``はclientが補います。
export type UpdateRequest = Pick<WasmRequest, "version" | "source"> & {
  sourceId?: WasmRequest["sourceId"];
  preprocess?: WasmRequest["preprocess"];
} & Settings<
    Pick<
      WasmRequest,
      "products" | "renderInputs" | "analysisOptions" | "renderPolicy" | "outputLimits"
    >
  >;

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
