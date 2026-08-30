import { execFile, spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
import { once } from "node:events";
import { tmpdir } from "node:os";
import { extname, join, normalize, resolve, sep } from "node:path";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { assertWasmArtifactSizes } from "./wasm-release-budget.mjs";
import {
  BROWSER_STARTUP_ATTEMPTS,
  BROWSER_STARTUP_ATTEMPT_TIMEOUT_MS,
  BROWSER_STARTUP_TOTAL_TIMEOUT_MS,
  retryBrowserStartup,
} from "./wasm-startup.mjs";
import {
  hostExecutableEnvironment,
  resolveHostExecutable,
} from "./host-executable.mjs";
import { hasExited, waitForExit } from "./process-lifecycle.mjs";

const run = promisify(execFile);
const POLL_INTERVAL_MS = 25;

if (
  process.argv[1]
  && pathToFileURL(resolve(process.argv[1])).href === import.meta.url
) {
  await main();
}

async function main() {
  const [packageInput, chromiumCommand = "chromium"] = process.argv.slice(2);
  if (!packageInput) {
    throw new Error("usage: wasm-release-smoke.mjs PACKAGE_DIRECTORY_OR_ARCHIVE [CHROMIUM]");
  }
  const input = resolve(packageInput);
  if ((await stat(input)).isDirectory()) {
    await runWasmPackageBrowserSmoke(input, chromiumCommand);
    return;
  }
  const chromium = await resolveHostExecutable(chromiumCommand);
  const root = await mkdtemp(join(tmpdir(), "adocweave-browser-smoke-"));
  try {
    await runArchiveSmoke(input, chromium, root);
  } finally {
    await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}

async function runArchiveSmoke(archive, chromium, root) {
  // npm packが作るtarballのrootは、versionを含まない``package``に正規化される。
  const { stdout: archiveList } = await run("tar", ["-tzf", resolve(archive)]);
  const members = archiveList.trimEnd().split("\n");
  const roots = new Set();
  for (const member of members) {
    if (member.startsWith("/") || member.split("/").includes("..")) {
      throw new Error(`unsafe archive member: ${member}`);
    }
    roots.add(member.split("/")[0]);
  }
  if (roots.size !== 1 || [...roots][0] !== "package") {
    throw new Error(`unexpected archive roots: ${[...roots].join(", ")}`);
  }
  await run("tar", ["-xzf", resolve(archive), "-C", root]);
  const entries = await import("node:fs/promises").then(({ readdir }) => readdir(root));
  if (entries.length !== 1 || entries[0] !== "package") {
    throw new Error(`unexpected archive root: ${entries.join(", ")}`);
  }
  const packageRoot = join(root, entries[0]);
  const archiveBytes = (await stat(archive)).size;
  await runPackageRootSmoke(packageRoot, chromium, root, archiveBytes);
}

export async function runWasmPackageBrowserSmoke(
  packageRoot,
  chromiumCommand = "chromium",
) {
  const chromium = await resolveHostExecutable(chromiumCommand);
  const root = await mkdtemp(join(tmpdir(), "adocweave-browser-smoke-"));
  try {
    await runPackageRootSmoke(resolve(packageRoot), chromium, root);
  } finally {
    await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}

async function runPackageRootSmoke(packageRoot, chromium, root, archiveBytes) {
  const wasmBytes = (await stat(join(packageRoot, "wasm/adocweave_wasm_bg.wasm"))).size;
  assertWasmArtifactSizes(archiveBytes ?? 0, wasmBytes);

  const requests = [];
  const server = createServer(async (request, response) => {
    try {
      const url = new URL(request.url, "http://localhost");
      requests.push(url.pathname);
      const requested = decodeURIComponent(url.pathname).replace(/^\/+/, "");
      const [context, ...segments] = requested.split("/");
      if (context !== "static") throw new Error("missing browser context prefix");
      const relative = segments.join("/") || "example/index.html";
      const path = normalize(join(packageRoot, relative));
      if (!path.startsWith(`${normalize(packageRoot)}${sep}`)) throw new Error("unsafe path");
      const types = { ".html": "text/html", ".mjs": "text/javascript", ".js": "text/javascript", ".wasm": "application/wasm" };
      response.setHeader("Content-Type", types[extname(path)] ?? "application/octet-stream");
      response.setHeader("Content-Security-Policy", "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; connect-src 'self'");
      if (relative === "example/trap-wasm.mjs") {
        response.end(`
          import { PROTOCOL_SCHEMA_VERSION } from "../worker/worker-protocol.mjs";
          let trapped = false;
          export default async function initialize() {}
          export function protocolSchemaVersion() { return PROTOCOL_SCHEMA_VERSION; }
          export function analyze() {
            if (!trapped) {
              trapped = true;
              throw new WebAssembly.RuntimeError("browser smoke trap");
            }
            return {};
          }
        `);
        return;
      }
      if (relative === "example/init-failure-wasm.mjs") {
        response.end(`
          export default async function initialize() { throw new Error("browser init failure"); }
          export function protocolSchemaVersion() { return 15; }
          export function analyze() { return {}; }
        `);
        return;
      }
      if (relative === "example/schema-failure-wasm.mjs") {
        response.end(`
          export default async function initialize() {}
          export function protocolSchemaVersion() { return 0; }
          export function analyze() { return {}; }
        `);
        return;
      }
      response.end(await readFile(path));
    } catch (error) {
      response.statusCode = 404;
      response.end(String(error));
    }
  });
  await new Promise((resolveListen) => server.listen(0, "127.0.0.1", resolveListen));
  const { port } = server.address();
  try {
    const url = `http://127.0.0.1:${port}/static/example/index.html?smoke=1`;
    console.log(`browser release smoke: starting non-isolated context with ${chromium}`);
    const state = await inspectPage(chromium, url, root);
    if (state.status !== "ready:2" || !state.html.includes("Latest browser result") || state.isolated) {
      throw new Error(`browser smoke failed; requests=${requests.join(",")}: ${JSON.stringify(state)}`);
    }
    if (!state.abortSettled) {
      throw new Error(`aborted analysis did not settle: ${JSON.stringify(state)}`);
    }
    if (!state.trapUsesFreshWorker) {
      throw new Error(`trapped WASM instance was reused: ${JSON.stringify(state)}`);
    }
    if (!state.initializationFailuresSettled) {
      throw new Error(`WASM initialization failures did not settle: ${JSON.stringify(state)}`);
    }
    const expectedAssets = [
      "/static/worker/worker.mjs",
      "/static/wasm/adocweave_wasm.js",
      "/static/wasm/adocweave_wasm_bg.wasm",
    ];
    if (expectedAssets.some((asset) => !requests.includes(asset))) {
      throw new Error(`browser assets were not requested: ${requests.join(",")}`);
    }
    if (Object.keys(state.products).length !== 1 || state.products.html !== true) {
      throw new Error(`browser result products mismatch: ${JSON.stringify(state)}`);
    }
    console.log("browser release smoke: passed non-isolated context");
  } finally {
    await new Promise((resolveClose) => server.close(resolveClose));
  }
  const sizeSummary = archiveBytes === undefined
    ? `wasm=${wasmBytes}`
    : `archive=${archiveBytes} wasm=${wasmBytes}`;
  console.log(`browser release smoke passed: ${sizeSummary}`);
}

export async function inspectPage(
  chromium,
  url,
  temporaryRoot,
  {
    attempts = BROWSER_STARTUP_ATTEMPTS,
    attemptTimeoutMs = BROWSER_STARTUP_ATTEMPT_TIMEOUT_MS,
    totalTimeoutMs = BROWSER_STARTUP_TOTAL_TIMEOUT_MS,
    dependencies,
  } = {},
) {
  return retryBrowserStartup(
    ({ remainingMs, signal }) => inspectPageAttempt(
      chromium,
      url,
      temporaryRoot,
      Math.min(attemptTimeoutMs, remainingMs),
      signal,
      dependencies,
    ),
    {
      attempts,
      totalTimeoutMs,
      onFailure: ({ attempt, attempts, elapsedMs, error, willRetry }) => {
        console.error(
          `browser release smoke: startup attempt ${attempt}/${attempts} failed after ${elapsedMs} ms; ${willRetry ? "retrying with a fresh profile" : "no retries remain"}:\n${error.message}`,
        );
      },
    },
  );
}

export async function inspectPageAttempt(
  chromium,
  url,
  temporaryRoot,
  startupTimeoutMs,
  signal,
  {
    randomUUID = () => crypto.randomUUID(),
    spawnBrowser = spawn,
    readText = readFile,
    fetchTarget = fetch,
    WebSocketImplementation = WebSocket,
    waitForBrowserExit = waitForExit,
  } = {},
) {
  const profile = join(temporaryRoot, `profile-${randomUUID()}`);
  const browser = spawnBrowser(chromium, [
    "--headless=new", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
    "--disable-background-networking", "--no-first-run", "--no-default-browser-check",
    "--remote-debugging-port=0", `--user-data-dir=${profile}`,
    "about:blank",
  ], {
    env: hostExecutableEnvironment(process.env),
    stdio: ["ignore", "ignore", "pipe"],
  });
  let spawnError;
  let stderr = "";
  let socket;
  let attemptTimer;
  let cleanupSignal = signal;
  browser.once("error", (error) => { spawnError = error; });
  browser.stderr.setEncoding("utf8");
  browser.stderr.on("data", (chunk) => { stderr = `${stderr}${chunk}`.slice(-8192); });
  try {
    const attemptController = new AbortController();
    let startupPhase = "DevToolsActivePort";
    attemptTimer = setTimeout(() => {
      const error = new Error(
        `Chromium startup attempt exceeded ${startupTimeoutMs} ms during ${startupPhase}`,
      );
      error.retryBrowserStartup = true;
      attemptController.abort(error);
    }, startupTimeoutMs);
    const startupSignal = AbortSignal.any([signal, attemptController.signal]);
    cleanupSignal = startupSignal;
    let port;
    let call;
    let event;
    try {
      port = await poll(async () => {
        const contents = await readText(
          join(profile, "DevToolsActivePort"),
          { encoding: "utf8", signal: startupSignal },
        );
        const candidate = Number.parseInt(contents.split("\n", 1)[0], 10);
        return Number.isInteger(candidate) && candidate > 0 ? candidate : undefined;
      }, () => browserFailure(browser, spawnError, stderr), startupTimeoutMs, startupSignal);
      startupPhase = "DevTools page target discovery";
      const target = await poll(async () => {
        const requestSignal = AbortSignal.any([
          startupSignal,
          AbortSignal.timeout(1000),
        ]);
        const response = await fetchTarget(
          `http://127.0.0.1:${port}/json/list`,
          { signal: requestSignal },
        );
        return (await response.json()).find((candidate) => candidate.type === "page");
      }, () => browserFailure(browser, spawnError, stderr), startupTimeoutMs, startupSignal);
      startupPhase = "DevTools WebSocket connection";
      socket = new WebSocketImplementation(target.webSocketDebuggerUrl);
      await once(socket, "open", { signal: startupSignal });
      let id = 0;
      const replies = new Map();
      const eventWaiters = new Map();
      socket.addEventListener("message", ({ data }) => {
        const message = JSON.parse(data);
        if (message.id && replies.has(message.id)) {
          const reply = replies.get(message.id);
          replies.delete(message.id);
          message.error ? reply.reject(new Error(message.error.message)) : reply.resolve(message.result);
        } else if (message.method && eventWaiters.has(message.method)) {
          eventWaiters.get(message.method)(message.params);
          eventWaiters.delete(message.method);
        }
      });
      call = (method, params = {}) => new Promise((resolveCall, rejectCall) => {
        const callId = ++id;
        replies.set(callId, { resolve: resolveCall, reject: rejectCall });
        socket.send(JSON.stringify({ id: callId, method, params }));
      });
      event = (method) => new Promise((resolveEvent) => eventWaiters.set(method, resolveEvent));
      startupPhase = "Page.enable";
      await withAbortSignal(call("Page.enable"), startupSignal);
    } catch (error) {
      const fatal = browserFailure(browser, spawnError, stderr);
      const failure = fatal ?? new Error(
        `browser did not complete the DevTools startup handshake: ${error.message}${stderr ? `\n${stderr}` : ""}`,
      );
      failure.retryBrowserStartup = fatal === undefined;
      throw failure;
    }
    clearTimeout(attemptTimer);
    attemptTimer = undefined;
    cleanupSignal = signal;

    const loaded = event("Page.loadEventFired");
    await withTimeout(call("Page.navigate", { url }), 5000, "Page.navigate timeout");
    await withTimeout(loaded, 20000, "page load timeout");
    const evaluated = await withTimeout(call("Runtime.evaluate", {
      expression: `new Promise((resolve, reject) => {
        const deadline = Date.now() + 15000;
        const wait = async () => {
          const status = document.querySelector('#status').value;
          if (status.startsWith('ready:') || status.startsWith('error:')) {
            const response = globalThis.adocweaveLastResult;
            const api = await import('/static/worker/index.mjs');
            const assets = api.defaultAssetUrls(
              new URL('/static/worker/index.mjs', location.href),
            );
            const trapClient = new api.AdocWeaveClient({
              ...assets,
              moduleUrl: new URL('/static/example/trap-wasm.mjs', location.href),
            });
            const trapCodes = [];
            try {
              for (let attempt = 0; attempt < 2; attempt += 1) {
                try {
                  await trapClient.analyze(
                    { source: '= trap' },
                    { signal: new AbortController().signal },
                  );
                } catch (error) {
                  trapCodes.push(error.code);
                }
              }
            } finally {
              trapClient.dispose();
            }
            const initializationCodes = [];
            for (const modulePath of [
              '/static/example/missing-wasm.mjs',
              '/static/example/init-failure-wasm.mjs',
              '/static/example/schema-failure-wasm.mjs',
            ]) {
              const failingClient = new api.AdocWeaveClient({
                ...assets,
                moduleUrl: new URL(modulePath, location.href),
              });
              try {
                await failingClient.analyze({ source: '= initialization failure' });
              } catch (error) {
                initializationCodes.push(error.code);
              } finally {
                failingClient.dispose();
              }
            }
            resolve({
              status,
              html: document.querySelector('#preview').textContent,
              isolated: crossOriginIsolated,
              abortSettled: globalThis.adocweaveAbortSettled === true,
              trapUsesFreshWorker: trapCodes.length === 2
                && trapCodes.every((code) => code === 'wasm-trapped'),
              initializationFailuresSettled: initializationCodes.length === 3
                && initializationCodes.every((code) => code === 'worker-failed'),
              products: Object.fromEntries(
                Object.keys(response).map((product) => [product, true]),
              ),
            });
          } else if (Date.now() >= deadline) {
            reject(new Error('result timeout: ' + status));
          } else setTimeout(() => wait().catch(reject), 25);
        };
        wait().catch(reject);
      })`,
      awaitPromise: true,
      returnByValue: true,
    }), 20000, "Runtime.evaluate timeout");
    socket.close();
    return evaluated.result.value;
  } finally {
    try {
      if (socket && socket.readyState < WebSocketImplementation.CLOSING) socket.close();
    } finally {
      try {
        await terminateBrowser(browser, cleanupSignal, waitForBrowserExit);
      } finally {
        clearTimeout(attemptTimer);
      }
    }
  }
}

async function terminateBrowser(browser, signal, waitForBrowserExit) {
  browser.kill("SIGTERM");
  if (signal?.aborted) {
    killAndDetach(browser);
    return;
  }
  try {
    if (await waitForBrowserExit(browser, 2000, { signal })) return;
  } catch (error) {
    if (!signal?.aborted) throw error;
    killAndDetach(browser);
    return;
  }
  browser.kill("SIGKILL");
  if (signal?.aborted) {
    detachBrowser(browser);
    return;
  }
  try {
    if (await waitForBrowserExit(browser, 5000, { signal })) return;
  } catch (error) {
    if (!signal?.aborted) throw error;
    detachBrowser(browser);
    return;
  }
  detachBrowser(browser);
  throw new Error("browser did not exit after SIGKILL");
}

function killAndDetach(browser) {
  browser.kill("SIGKILL");
  detachBrowser(browser);
}

function detachBrowser(browser) {
  browser.stderr?.destroy?.();
  browser.unref?.();
}

async function withAbortSignal(promise, signal) {
  if (!signal) return promise;
  signal.throwIfAborted();
  let onAbort;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        onAbort = () => reject(signal.reason);
        signal.addEventListener("abort", onAbort, { once: true });
      }),
    ]);
  } finally {
    signal.removeEventListener("abort", onAbort);
  }
}

async function poll(operation, failure, timeoutMs, signal) {
  let error;
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    signal?.throwIfAborted();
    const fatal = failure?.();
    if (fatal) throw fatal;
    try {
      const value = await withAbortSignal(operation(), signal);
      if (value) return value;
    } catch (caught) {
      signal?.throwIfAborted();
      error = caught;
    }
    await abortableDelay(POLL_INTERVAL_MS, signal);
  }
  signal?.throwIfAborted();
  throw error ?? new Error(
    `Chromium did not become ready within ${timeoutMs} ms`,
  );
}

async function abortableDelay(milliseconds, signal) {
  signal?.throwIfAborted();
  let timer;
  let onAbort;
  try {
    await new Promise((resolveDelay, rejectDelay) => {
      timer = setTimeout(resolveDelay, milliseconds);
      if (signal) {
        onAbort = () => rejectDelay(signal.reason);
        signal.addEventListener("abort", onAbort, { once: true });
      }
    });
  } finally {
    clearTimeout(timer);
    if (onAbort) signal.removeEventListener("abort", onAbort);
  }
}

function browserFailure(browser, spawnError, stderr) {
  if (spawnError) return new Error(`browser failed to start: ${spawnError.message}`);
  if (!hasExited(browser)) return undefined;
  const status = browser.signalCode ?? browser.exitCode;
  return new Error(`browser exited before DevTools became ready (${status})${stderr ? `:\n${stderr}` : ""}`);
}

async function withTimeout(promise, milliseconds, message) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(message)), milliseconds);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
