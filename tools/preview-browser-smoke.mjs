// Real-browser coverage of the ordinary HTML preview's CSP and live updates.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { connectCdp } from "./browser-cdp.mjs";
import { hostExecutableEnvironment, resolveHostExecutable } from "./host-executable.mjs";

const [binaryArgument, browserArgument] = process.argv.slice(2);
assert.ok(binaryArgument && browserArgument, "usage: node tools/preview-browser-smoke.mjs CLI CHROMIUM");
const binary = resolve(binaryArgument);
const browser = await resolveHostExecutable(browserArgument);
const root = await mkdtemp(join(tmpdir(), "adocweave-preview-browser-"));
const children = [];
let socket;
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
async function poll(probe) {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    const value = await probe();
    if (value) return value;
    await delay(50);
  }
  throw new Error("ordinary preview browser probe timed out");
}
function child(executable, args) {
  const process = spawn(executable, args, { cwd: root, env: hostExecutableEnvironment(globalThis.process.env), stdio: ["ignore", "ignore", "pipe"] });
  children.push(process);
  return process;
}
async function endpoint(process, pattern) {
  let stderr = "";
  process.stderr.on("data", bytes => { stderr += bytes; });
  return poll(() => {
    assert.equal(process.exitCode, null, `process exited: ${stderr}`);
    return stderr.match(pattern)?.[1];
  });
}
try {
  const manuscript = join(root, "talk.adoc");
  await writeFile(manuscript, "= Preview\n\nBEFORE_UPDATE\n");
  const server = child(binary, ["preview", "talk.adoc", "--port", "0"]);
  const address = await endpoint(server, /AdocWeave preview: (http:\/\/[^\s]+)/);
  const chromium = child(browser, ["--headless=new", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage", "--disable-background-networking", "--no-first-run", "--no-default-browser-check", "--remote-debugging-port=0", `--user-data-dir=${join(root, "profile")}`, "about:blank"]);
  const browserSocket = await endpoint(chromium, /DevTools listening on (ws:\/\/[^\s]+)/);
  const targets = await fetch(`http://${new URL(browserSocket).host}/json/list`, { signal: AbortSignal.timeout(5000) }).then(response => response.json());
  socket = new WebSocket(targets.find(target => target.type === "page").webSocketDebuggerUrl);
  let pausedPoll;
  const violations = [];
  const cdp = await connectCdp(socket, { onEvent(event) {
    if (event.method === "Fetch.requestPaused") pausedPoll = event.params.requestId;
    if (event.method === "Log.entryAdded" && event.params.entry.text.includes("Content Security Policy")) violations.push(event.params.entry.text);
  } });
  await cdp.call("Page.enable");
  await cdp.call("Runtime.enable");
  await cdp.call("Log.enable");
  await cdp.call("Fetch.enable", { patterns: [{ urlPattern: "*/events", requestStage: "Request" }] });
  const evaluate = async expression => {
    const result = await cdp.call("Runtime.evaluate", { expression, returnByValue: true });
    assert.equal(result.exceptionDetails, undefined, JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  await cdp.call("Page.navigate", { url: address });
  await poll(() => pausedPoll);
  assert.equal(await evaluate('document.querySelector(\'meta[name="adocweave-preview-generation"]\').content'), "1");
  // Rebuild after HTML delivery but before the first poll response: the page
  // must compare against its delivered generation, not accept the new one.
  await writeFile(manuscript, "= Preview\n\nAFTER_UPDATE\n");
  await poll(async () => (await fetch(`${address}events`, { signal: AbortSignal.timeout(5000) }).then(response => response.json())).generation >= 2);
  await cdp.call("Fetch.continueRequest", { requestId: pausedPoll });
  await cdp.call("Fetch.disable");
  await poll(async () => (await cdp.call("Page.captureSnapshot", { format: "mhtml" })).data.includes("AFTER_UPDATE"));
  await writeFile(manuscript, "= Preview\n\ninclude::missing.adoc[]\n");
  await poll(async () => (await evaluate("document.querySelector('pre')?.textContent ?? ''")).includes("missing.adoc"));
  await writeFile(join(root, "missing.adoc"), "RECOVERED_INCLUDE\n");
  await poll(async () => (await cdp.call("Page.captureSnapshot", { format: "mhtml" })).data.includes("RECOVERED_INCLUDE"));
  await poll(async () => (await evaluate("document.querySelector('pre')?.textContent ?? ''")).trim() === "[]");
  assert.deepEqual(violations, [], "preview polling must satisfy its real response CSP");
  cdp.check();
  console.log("Ordinary HTML preview: initial-poll race, rendered updates, diagnostics, and recovery passed.");
} finally {
  socket?.close();
  for (const process of children.reverse()) {
    if (process.exitCode !== null || process.signalCode !== null) continue;
    const exited = once(process, "exit");
    process.kill("SIGTERM");
    await Promise.race([exited, delay(2000)]);
    if (process.exitCode === null && process.signalCode === null) { process.kill("SIGKILL"); await exited; }
  }
  await rm(root, { recursive: true, force: true });
}
