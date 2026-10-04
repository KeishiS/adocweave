// Real fixed-runtime regression: file URLs, managed HTTP, fragments and notes.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { hostExecutableEnvironment } from "./host-executable.mjs";

const [binaryArgument, chromiumArgument] = process.argv.slice(2);
assert.ok(binaryArgument && chromiumArgument, "usage: node tools/slides-browser-smoke.mjs CLI CHROMIUM");
const binary = resolve(binaryArgument);
const chromium = resolve(chromiumArgument);
const root = await mkdtemp(join(tmpdir(), "adocweave-slides-smoke-"));
const environment = hostExecutableEnvironment(process.env);
const children = [];
const sockets = [];
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
async function poll(callback) {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    const value = await callback();
    if (value) return value;
    await delay(25);
  }
  throw new Error("slides browser probe deadline");
}
function child(executable, arguments_, env = environment) {
  const process_ = spawn(executable, arguments_, { cwd: root, env, stdio: ["ignore", "ignore", "pipe"] });
  children.push(process_);
  let error;
  let stderr = "";
  process_.stderr.setEncoding("utf8");
  process_.stderr.on("data", text => { stderr = (stderr + text).slice(-8192); });
  process_.on("error", value => { error = value; });
  return { process_, check() { if (error) throw error; if (process_.exitCode !== null) throw new Error(stderr); }, stderr() { return stderr; } };
}
async function server(audience) {
  // Serving must work without any Node executable in PATH.
  const running = child(binary, ["serve", join(root, audience), "--port", "0"], { ...environment, PATH: "", ADOCWEAVE_SLIDES_HELPER: "/missing-helper" });
  const address = await poll(() => { running.check(); return running.stderr().match(/http:\/\/127\.0\.0\.1:(\d+)\//)?.[1]; });
  return `http://127.0.0.1:${address}`;
}
async function connect(target, ports) {
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  sockets.push(socket);
  await once(socket, "open");
  let id = 0;
  const pending = new Map();
  const errors = [];
  const blocked = [];
  function call(method, params = {}) {
    return new Promise((resolve, reject) => { const requestId = ++id; pending.set(requestId, { resolve, reject }); socket.send(JSON.stringify({ id: requestId, method, params })); });
  }
  socket.addEventListener("message", async ({ data }) => {
    const message = JSON.parse(data);
    if (message.id) {
      const request = pending.get(message.id);
      if (request) { pending.delete(message.id); message.error ? request.reject(new Error(message.error.message)) : request.resolve(message.result); }
    } else if (message.method === "Runtime.exceptionThrown") errors.push(message.params.exceptionDetails);
    else if (message.method === "Log.entryAdded" && message.params.entry.level === "error") errors.push(message.params.entry);
    else if (message.method === "Fetch.requestPaused") {
      const { requestId, request } = message.params;
      const url = new URL(request.url);
      const allowed = url.protocol === "file:" || url.protocol === "about:" || (url.protocol === "http:" && url.hostname === "127.0.0.1" && ports.has(url.port));
      if (!allowed) blocked.push(request.url);
      await call(allowed ? "Fetch.continueRequest" : "Fetch.failRequest", allowed ? { requestId } : { requestId, errorReason: "BlockedByClient" });
    }
  });
  async function evaluate(expression, userGesture = false) {
    const result = await call("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true, userGesture });
    assert.ok(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
    return result.result.value;
  }
  await call("Page.enable");
  await call("Runtime.enable");
  await call("Log.enable");
  await call("Network.enable");
  await call("Network.setCacheDisabled", { cacheDisabled: true });
  await call("Fetch.enable", { patterns: [{ urlPattern: "*" }] });
  await call("Page.addScriptToEvaluateOnNewDocument", { source: "window.probeReady=false;window.probeViolations=[];document.addEventListener('securitypolicyviolation',e=>probeViolations.push(e.effectiveDirective));window.addEventListener('adocweave-slides-ready',()=>probeReady=true);" });
  // A newly opened speaker window may have loaded before CDP attached.
  await evaluate("window.probeViolations ??= [];document.addEventListener('securitypolicyviolation',e=>probeViolations.push(e.effectiveDirective))");
  return { call, evaluate, errors, blocked };
}
try {
  await writeFile(join(root, "talk.adoc"), `= Reference regression
:lang: ja

[#method]
== Method — 手法

The heading reference <<method>> preserves an already visible stage.

[%step]
* [[first-step]]First stage.
* [[second-step]]Second stage.

[.notes]
--
PRIVATE_NOTE
--

[#vertical]
== Vertical

The heading reference <<vertical>> preserves an already visible stage.

[%step]
* [[vertical-first]]First vertical stage.
* [[vertical-second]]Second vertical stage.

[#detail]
=== Detail

The heading reference <<detail>> preserves an already visible stage.

[%step]
* [[detail-first]]First detail stage.
* [[detail-second]]Second detail stage.

[#last]
== Last

Return to <<method>>, <<first-step>>, or <<second-step>>;
<<vertical>>, <<vertical-first>>, or <<vertical-second>>;
<<detail>>, <<detail-first>>, or <<detail-second>>.
`);
  for (const audience of ["public", "presenter"]) {
    const conversion = spawnSync(binary, ["convert", "talk.adoc", "--to", "revealjs", "--output", audience, "--audience", audience], { cwd: root, env: environment, encoding: "utf8" });
    assert.equal(conversion.status, 0, conversion.stderr);
    const manifest = JSON.parse(await readFile(join(root, audience, ".adocweave-manifest.json"), "utf8"));
    for (const file of manifest.files) {
      const bytes = await readFile(join(root, audience, file.path));
      assert.equal(bytes.length, file.sizeBytes);
      assert.equal(createHash("sha256").update(bytes).digest("hex"), file.sha256);
    }
    const html = await readFile(join(root, audience, "index.html"), "utf8");
    assert.equal(html.includes("PRIVATE_NOTE"), audience === "presenter");
    assert.equal(manifest.files.some(file => file.path === "assets/notes.js"), audience === "presenter");
  }
  const publicServer = await server("public");
  const presenterServer = await server("presenter");
  const ports = new Set([new URL(publicServer).port, new URL(presenterServer).port]);
  const browser = child(chromium, ["--headless=new", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage", "--disable-background-networking", "--no-first-run", "--no-default-browser-check", "--disable-popup-blocking", "--remote-debugging-port=0", `--user-data-dir=${join(root, "profile")}`, "about:blank"]);
  const debuggingPort = await poll(async () => {
    browser.check();
    try { return Number.parseInt((await readFile(join(root, "profile", "DevToolsActivePort"), "utf8")).split("\n")[0], 10); } catch { return false; }
  });
  const targets = async () => (await fetch(`http://127.0.0.1:${debuggingPort}/json/list`)).json();
  const target = await poll(async () => (await targets()).find(target => target.type === "page"));
  const page = await connect(target, ports);
  for (const url of [pathToFileURL(join(root, "public", "index.html")).href, `${publicServer}/`, `${presenterServer}/`]) {
    await page.call("Page.navigate", { url });
    await poll(() => page.evaluate(`location.href.split('#')[0]===${JSON.stringify(url)}&&probeReady`));
    assert.equal(await page.evaluate("Reveal.getTotalSlides()"), 5);
    for (const [heading, first, second, h, v] of [["method", "first-step", "second-step", 1, 0], ["vertical", "vertical-first", "vertical-second", 2, 0], ["detail", "detail-first", "detail-second", 2, 1]]) {
      // Returning backwards invokes the real Reveal hash-click listener as well.
      await page.evaluate(`Reveal.slide(${h},${v},0);Reveal.slide(3,0,-1);document.querySelector('#last a[href="#${heading}"]').click()`);
      assert.deepEqual(await page.evaluate("Reveal.getIndices()"), { h, v, f: -1 });
      for (const [id, stage] of [[first, 0], [second, 1]]) {
        await page.evaluate(`Reveal.slide(3,0,-1);document.querySelector('#last a[href="#${id}"]').click()`);
        assert.deepEqual(await page.evaluate("Reveal.getIndices()"), { h, v, f: stage });
      }
      await page.evaluate(`document.querySelector('#${heading} a[href="#${heading}"]').click()`);
      assert.deepEqual(await page.evaluate("Reveal.getIndices()"), { h, v, f: 1 });
      await poll(() => page.evaluate(`location.hash==='#/${heading}/1'`));
      await page.evaluate("window.reloadMarker=true");
      await page.call("Page.reload", { ignoreCache: true });
      await poll(() => page.evaluate("!window.reloadMarker&&probeReady"));
      assert.deepEqual(await page.evaluate("Reveal.getIndices()"), { h, v, f: 1 });
    }
    assert.deepEqual(await page.evaluate("probeViolations"), []);
    assert.equal(await page.evaluate("document.querySelectorAll('iframe').length"), 0);
  }
  await page.evaluate("Reveal.slide(1,0,1)");
  await page.evaluate("Reveal.getPlugin('notes').open()", true);
  const popupTarget = await poll(async () => (await targets()).find(candidate => candidate.type === "page" && candidate.id !== target.id));
  const popup = await connect(popupTarget, ports);
  await poll(() => popup.evaluate("document.body?.textContent.includes('PRIVATE_NOTE')&&[...document.querySelectorAll('iframe')].length===2&&[...document.querySelectorAll('iframe')].every(frame=>frame.contentWindow.Reveal?.isReady())"));
  assert.deepEqual(await popup.evaluate("probeViolations"), []);
  for (const connection of [page, popup]) {
    assert.deepEqual(connection.blocked, []);
    assert.deepEqual(connection.errors.filter(error => !(error.source === "network" && error.url?.endsWith("/favicon.ico") && error.text?.includes("404"))), []);
  }
  console.log("slides browser smoke passed: file/managed HTTP, target fragment stages, reload, notes, CSP, offline");
} finally {
  for (const socket of sockets) socket.close();
  for (const process_ of children.reverse()) {
    if (process_.exitCode !== null) continue;
    const exited = once(process_, "exit");
    process_.kill("SIGTERM");
    await Promise.race([exited, delay(2000)]);
    if (process_.exitCode === null) { process_.kill("SIGKILL"); await Promise.race([exited, delay(2000)]); }
  }
  await rm(root, { recursive: true, force: true });
}
