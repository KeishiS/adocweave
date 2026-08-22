import type { WasmError, WasmRequest, WasmResponse } from "./protocol.d.mts";

export declare const PROTOCOL_SCHEMA_VERSION: 14;
export declare const WORKER_PROTOCOL_VERSION: 2;
export declare const PACKAGE_VERSION: string;

export type WorkerRequest =
  | {
      type: "initialize";
      protocolVersion: number;
      moduleUrl: string;
      wasmUrl: string;
      debounceMs: number;
      cancellationBuffer: SharedArrayBuffer | null;
    }
  | {
      type: "analyze";
      protocolVersion: number;
      version: number;
      generation: number;
      payload: WasmRequest;
    };

export type WorkerResponse =
  | { type: "ready"; protocolVersion: number }
  | {
      type: "result";
      protocolVersion: number;
      version: number;
      generation: number;
      result: WasmResponse;
    }
  | {
      type: "error";
      protocolVersion: number;
      version: number;
      generation: number;
      error: WasmError;
    };

export interface AdocWeaveError {
  code: string;
  message: string;
  sourceVersion: number | null;
  generation: number;
}

export declare const WORKER_MESSAGE_FIELDS: Record<string, readonly string[]>;

export declare function validateWorkerMessage(
  value: unknown,
  direction: "requests" | "responses",
): value is WorkerRequest | WorkerResponse;

export declare function validateClientError(value: unknown): value is AdocWeaveError;
