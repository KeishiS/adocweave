import { analysisPayload, parseWasmError } from "./analysis.mjs";
import { WORKER_PROTOCOL_VERSION, validateWorkerMessage } from "./worker-protocol.mjs";

export class AdocWeaveClient {
  #options;
  #worker = null;
  #workerListeners = null;
  #ready = null;
  #readyReject = null;
  #active = null;
  #nextRequestId = 0;
  #disposed = false;

  constructor({
    workerUrl,
    moduleUrl,
    wasmUrl,
    Worker: WorkerConstructor = globalThis.Worker,
  }) {
    this.#options = {
      workerUrl: String(workerUrl),
      moduleUrl: String(moduleUrl),
      wasmUrl: String(wasmUrl),
      WorkerConstructor,
    };
  }

  analyze(request, { signal } = {}) {
    if (this.#disposed) {
      return Promise.reject(new AdocWeaveError({
        code: "disposed",
        message: "AdocWeaveClient was disposed",
      }));
    }
    if (this.#active !== null) {
      return Promise.reject(new AdocWeaveError({
        code: "analysis-in-progress",
        message: "an analysis is already in progress",
      }));
    }
    if (signal?.aborted) return Promise.reject(abortError());

    const requestId = this.#allocateRequestId();
    let payload;
    try {
      payload = analysisPayload(request);
    } catch (cause) {
      const requestError = parseWasmError(cause);
      return Promise.reject(new AdocWeaveError(requestError ?? {
        code: "invalid-request",
        message: "the analysis request is invalid",
      }));
    }

    return new Promise((resolve, reject) => {
      const active = {
        requestId,
        worker: null,
        signal,
        abortListener: null,
        resolve,
        reject,
      };
      if (signal !== undefined) {
        active.abortListener = () => {
          if (this.#active !== active) return;
          const error = abortError();
          this.#settleActive("reject", error);
          this.#terminateWorker(active.worker ?? this.#worker, error);
        };
        signal.addEventListener("abort", active.abortListener, { once: true });
      }
      this.#active = active;

      this.#ensureWorker().then((worker) => {
        if (this.#active !== active || worker !== this.#worker) return;
        active.worker = worker;
        try {
          worker.postMessage({
            type: "analyze",
            requestId,
            payload,
          });
        } catch (cause) {
          if (cause?.name === "DataCloneError") {
            this.#settleActive("reject", new AdocWeaveError({
              code: "invalid-request",
              message: "the analysis request must be structured-cloneable",
            }));
          } else {
            this.#failWorker(cause, worker);
          }
        }
      }).catch((error) => {
        if (this.#active !== active) return;
        this.#settleActive("reject", asClientError(error));
      });
    });
  }

  dispose() {
    if (this.#disposed) return;
    this.#disposed = true;
    const error = new AdocWeaveError({
      code: "disposed",
      message: "AdocWeaveClient was disposed",
    });
    this.#settleActive("reject", error);
    this.#terminateWorker(this.#worker, error);
  }

  #allocateRequestId() {
    this.#nextRequestId = this.#nextRequestId === 0xffff_ffff
      ? 1
      : this.#nextRequestId + 1;
    return this.#nextRequestId;
  }

  #ensureWorker() {
    if (this.#worker !== null) return this.#ready;
    return this.#spawnWorker();
  }

  #spawnWorker() {
    let worker;
    try {
      if (typeof this.#options.WorkerConstructor !== "function") {
        throw new TypeError("a Worker constructor is required");
      }
      worker = new this.#options.WorkerConstructor(this.#options.workerUrl, {
        type: "module",
      });
    } catch (cause) {
      return Promise.reject(workerError(cause));
    }

    this.#worker = worker;
    const ready = new Promise((resolve, reject) => {
      this.#readyReject = reject;
      const onMessage = ({ data }) => this.#handleMessage(worker, data, resolve);
      const onError = (event) => {
        if (worker !== this.#worker) return;
        this.#failWorker(event.message || "AdocWeave worker failed", worker);
      };
      const onMessageError = () => {
        if (worker !== this.#worker) return;
        this.#failWorker("AdocWeave worker returned an unreadable response", worker);
      };
      this.#workerListeners = { worker, onMessage, onError, onMessageError };
      worker.addEventListener("message", onMessage);
      worker.addEventListener("error", onError);
      worker.addEventListener("messageerror", onMessageError);
    });
    this.#ready = ready;
    ready.catch(() => {});

    try {
      worker.postMessage({
        protocolVersion: WORKER_PROTOCOL_VERSION,
        type: "init",
        moduleUrl: this.#options.moduleUrl,
        wasmUrl: this.#options.wasmUrl,
      });
    } catch (cause) {
      this.#failWorker(cause, worker);
    }
    return ready;
  }

  #handleMessage(worker, data, resolveReady) {
    if (worker !== this.#worker || this.#disposed) return;
    if (!validateWorkerMessage(data, "responses")) {
      this.#failTerminal(new AdocWeaveError({
        code: "worker-failed",
        message: "worker returned a response outside the internal protocol",
      }), worker);
      return;
    }

    if (data.type === "ready") {
      if (this.#readyReject === null) {
        this.#failTerminal(new AdocWeaveError({
          code: "worker-failed",
          message: "worker became ready outside initialization",
        }), worker);
        return;
      }
      if (data.protocolVersion !== WORKER_PROTOCOL_VERSION) {
        this.#failTerminal(new AdocWeaveError({
          code: "unsupported-worker-protocol",
          message: `expected worker protocol ${WORKER_PROTOCOL_VERSION}`,
        }), worker);
        return;
      }
      this.#readyReject = null;
      resolveReady(worker);
      return;
    }

    if (data.type === "initialization-error") {
      if (this.#readyReject === null) {
        this.#failTerminal(new AdocWeaveError({
          code: "worker-failed",
          message: "worker reported initialization failure after becoming ready",
        }), worker);
        return;
      }
      this.#failTerminal(new AdocWeaveError(data.error), worker);
      return;
    }

    const active = this.#active;
    if (active === null || active.worker !== worker) {
      this.#failTerminal(new AdocWeaveError({
        code: "worker-failed",
        message: "worker returned a response without an active request",
      }), worker);
      return;
    }
    if (data.requestId !== active.requestId) {
      this.#failTerminal(new AdocWeaveError({
        code: "worker-failed",
        message: "worker response requestId does not match its request",
      }), worker);
      return;
    }

    if (data.type === "result") {
      this.#settleActive("resolve", data.result);
    } else if (data.type === "error") {
      this.#settleActive("reject", new AdocWeaveError(data.error));
    } else {
      this.#failTerminal(new AdocWeaveError(data.error), worker);
    }
  }

  #failWorker(cause, worker) {
    this.#failTerminal(workerError(cause), worker);
  }

  #failTerminal(error, worker) {
    if (worker !== this.#worker) {
      this.#detachWorkerListeners(worker);
      worker?.terminate();
      return;
    }
    this.#settleActive("reject", error);
    this.#terminateWorker(worker, error);
  }

  #settleActive(action, value) {
    const active = this.#active;
    if (active === null) return;
    this.#active = null;
    if (active.signal !== undefined && active.abortListener !== null) {
      active.signal.removeEventListener("abort", active.abortListener);
    }
    active[action](value);
  }

  #terminateWorker(worker = this.#worker, readyError = null) {
    if (worker === null) return;
    if (worker !== this.#worker) {
      this.#detachWorkerListeners(worker);
      worker.terminate();
      return;
    }
    if (readyError !== null) this.#readyReject?.(readyError);
    this.#readyReject = null;
    this.#detachWorkerListeners(worker);
    worker.terminate();
    this.#worker = null;
    this.#ready = null;
  }

  #detachWorkerListeners(worker) {
    const listeners = this.#workerListeners;
    if (listeners === null || listeners.worker !== worker) return;
    worker.removeEventListener?.("message", listeners.onMessage);
    worker.removeEventListener?.("error", listeners.onError);
    worker.removeEventListener?.("messageerror", listeners.onMessageError);
    this.#workerListeners = null;
  }
}

export class AdocWeaveError extends Error {
  constructor({ code, message }) {
    super(message);
    this.name = "AdocWeaveError";
    this.code = code;
  }
}

const LIFECYCLE_ERROR_CODES = new Set([
  "cancelled",
  "analysis-in-progress",
  "disposed",
  "unsupported-worker-protocol",
  "wasm-trapped",
  "worker-failed",
]);

export function isAdocWeaveLifecycleError(error) {
  return error instanceof AdocWeaveError &&
    LIFECYCLE_ERROR_CODES.has(error.code);
}

function workerError(cause) {
  return new AdocWeaveError({
    code: "worker-failed",
    message: cause instanceof Error ? cause.message : String(cause),
  });
}

function asClientError(error) {
  return error instanceof AdocWeaveError ? error : workerError(error);
}

function abortError() {
  return new AdocWeaveError({
    code: "cancelled",
    message: "The analysis was cancelled",
  });
}
