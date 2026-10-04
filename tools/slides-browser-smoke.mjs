// Real fixed-runtime regression: file URLs, managed HTTP, fragments and notes.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { hostExecutableEnvironment, resolveHostExecutable } from "./host-executable.mjs";

const [binaryArgument, chromiumArgument, helperArgument, artifactArgument] = process.argv.slice(2);
assert.ok(binaryArgument && chromiumArgument, "usage: node tools/slides-browser-smoke.mjs CLI CHROMIUM [HELPER_MJS [ARTIFACT_DIRECTORY]]");
const binary = resolve(binaryArgument);
const chromium = await resolveHostExecutable(chromiumArgument);
const helper = helperArgument && resolve(helperArgument);
const artifacts = helper && resolve(artifactArgument ?? "target/slides-browser");
const reports = [];
let publicBody;
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
  const researchArguments = [];
  if (helper) {
    for (const file of ["research.adoc", "references.json", "result.svg", "research.css", "research.csl"]) {
      await writeFile(join(root, file === "research.adoc" ? "talk.adoc" : file),
        await readFile(new URL(`../fixtures/slides-browser/${file}`, import.meta.url)));
    }
    for (const [source, destination] of [["locale-en-US.xml", "locale.xml"]]) {
      await writeFile(join(root, destination), await readFile(new URL(`../packages/slides-helper/fixtures/${source}`, import.meta.url)));
    }
    researchArguments.push("--slides-helper", helper, "--bibliography", "references.json", "--csl-style", "research.csl",
      "--csl-locale", "locale.xml", "--css", "research.css");
    await mkdir(artifacts, { recursive: true });
  }
  for (const audience of ["public", "presenter"]) {
    const started = performance.now();
    const conversion = spawnSync(binary, ["convert", "talk.adoc", "--to", "revealjs", "--output", audience, "--audience", audience,
      ...researchArguments], { cwd: root, env: environment, encoding: "utf8" });
    assert.equal(conversion.status, 0, conversion.stderr);
    if (helper) reports.push({ name: `convert-${audience}`, milliseconds: performance.now() - started });
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
    assert.equal(await page.evaluate("Reveal.getTotalSlides()"), helper ? 6 : 5);
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
    if (helper) {
      const presenter = url === `${presenterServer}/`;
      await page.evaluate("Reveal.slide(1,0,1)");
      const content = await page.evaluate(`(() => {
        const math = [...document.querySelectorAll('.math-rendered')];
        const bodyMath = math.filter(node => !node.closest('aside.notes'));
        const notesMath = math.filter(node => node.closest('aside.notes'));
        const ids = [...document.querySelectorAll('[id]')].map(node => node.id);
        const missing = [...document.querySelectorAll('[href^="#"]')].map(node => node.getAttribute('href').slice(1))
          .filter(id => id && !id.startsWith('/') && !document.getElementById(id));
        const text = selector => document.querySelector(selector)?.textContent.replace(/\\s+/g, '') ?? '';
        const label = selector => [...((typeof selector === 'string' ? document.querySelector(selector) : selector)
          ?.querySelectorAll('[data-mml-node="mtext"] use[data-c]') ?? [])]
          .map(node => String.fromCodePoint(Number.parseInt(node.getAttribute('data-c'), 16))).join('');
        const credits = document.querySelector('.slides-attribution');
        const rectangle = credits.getBoundingClientRect();
        return {
          bodyMath: bodyMath.length, notesMath: notesMath.length,
          svg: math.filter(node => node.querySelector('svg path, svg text')).length,
          matrix: document.querySelectorAll('#vertical svg [data-mml-node="mtable"] [data-mml-node="mtr"]').length,
          tableMath: document.querySelectorAll('#detail td .math-rendered').length,
          tableCitation: document.querySelectorAll('#detail td .citation').length,
          duplicateIds: ids.length - new Set(ids).size, missing,
          footnote: label('#method > .footnotes li .math-rendered'),
          notesFootnote: label('#method aside.notes .footnotes li .math-rendered'),
          notesResult: label(notesMath.find(node => node.closest('#method') &&
            node.querySelector('.math-source')?.textContent.includes('label{result}'))),
          citation: text('#method p .citation'),
          bodyReferences: [...document.querySelectorAll('.bibliography-anchor[id^="slides-body-bib-"]')].map(node => node.id),
          notesReferences: [...document.querySelectorAll('.bibliography-anchor[id^="slides-notes-bib-"]')].map(node => node.id),
          privateText: document.body.textContent.includes('PRIVATE_BIBLIOGRAPHY'),
          imageReady: [...document.images].every(image => image.complete && image.naturalWidth > 0),
          customSize: getComputedStyle(document.querySelector('.reveal')).fontSize,
          creditsVisible: rectangle.width > 0 && rectangle.height > 0 && rectangle.bottom <= innerHeight + 1,
          credits: credits.textContent,
          browserMathJax: [...document.scripts].some(script => /mathjax|citeproc/i.test(script.src)),
          body: [...document.querySelectorAll('.math-rendered, .citation, .footnotes li, #slides-body-references li, figcaption')]
            .filter(node => !node.closest('aside.notes'))
            .map(node => [node.className, node.textContent.replace(/\\s+/g, ' ').trim(), node.querySelector('svg')?.outerHTML ?? '']),
        };
      })()`);
      assert.equal(content.bodyMath, 10);
      assert.equal(content.notesMath, presenter ? 6 : 0);
      assert.equal(content.svg, presenter ? 16 : 10);
      assert.equal(content.matrix, 2);
      assert.equal(content.tableMath, 1);
      assert.equal(content.tableCitation, 1);
      assert.equal(content.duplicateIds, 0);
      assert.deepEqual(content.missing, []);
      assert.equal(content.footnote, "(2)");
      assert.equal(content.citation, "(Doe2024b,pp.12–15)↗");
      assert.equal(content.bodyReferences.length, 3);
      assert.equal(content.notesReferences.length, presenter ? 2 : 0);
      assert.equal(content.privateText, presenter);
      if (presenter) {
        assert.equal(content.notesFootnote, "(1)");
        assert.equal(content.notesResult, "(2)");
      }
      assert.equal(content.imageReady, true);
      assert.equal(content.customSize, "24px");
      assert.equal(content.creditsVisible, true);
      assert.match(content.credits, /Frank Bennett/);
      assert.match(content.credits, /citeproc-js implements the Citation Style Language/);
      assert.ok(content.credits.includes("https://citationstyles.org/"));
      assert.equal(content.browserMathJax, false);
      if (publicBody) assert.deepEqual(content.body, publicBody);
      else publicBody = content.body;
      await page.evaluate("document.querySelector('#method .citation-link').click()");
      assert.deepEqual(await page.evaluate("Reveal.getIndices()"), { h: 4, v: 0 });
      await page.evaluate("document.querySelector('#slides-body-bib-zebra').closest('li').querySelector('.bibliography-backref').click()");
      assert.deepEqual(await page.evaluate("Reveal.getIndices()"), { h: 1, v: 0, f: -1 });
      await page.evaluate(`Reveal.slide(3,0,-1);document.querySelector('#last p .math-rendered svg a')
        .dispatchEvent(new MouseEvent('click',{bubbles:true,cancelable:true}))`);
      assert.deepEqual(await page.evaluate("Reveal.getIndices()"), { h: 1, v: 0, f: -1 });
      await page.evaluate("Reveal.slide(1,0,1);document.querySelector('#method .footnote-ref').click();document.querySelector('#method .footnote-backref').click()");
      assert.deepEqual(await page.evaluate("Reveal.getIndices()"), { h: 1, v: 0, f: 1 });
      const fonts = {};
      // Chromium's CSS font inspection itself raises opaque-origin errors for
      // file pages. Inspect the identical bundle's glyphs over managed HTTP;
      // file navigation and rendering keep their independent console checks.
      if (!url.startsWith("file:")) {
        await page.call("DOM.enable");
        await page.call("CSS.enable");
        const { root: document_ } = await page.call("DOM.getDocument");
        for (const [kind, selector] of [["dom", "#method h1, #method h2"], ["svg", "#method .math-rendered svg text"]]) {
          const { nodeId } = await page.call("DOM.querySelector", { nodeId: document_.nodeId, selector });
          assert.ok(nodeId, `Missing CJK ${kind} probe node.`);
          fonts[kind] = (await page.call("CSS.getPlatformFontsForNode", { nodeId })).fonts;
          assert.ok(fonts[kind].some(font => /Noto.*CJK/.test(font.familyName) && font.glyphCount >= 2 && !font.isCustomFont),
            `Missing system CJK glyphs in ${kind}: ${JSON.stringify(fonts[kind])}`);
        }
      }
      const name = url.startsWith("file:") ? "public-file" : presenter ? "presenter-http" : "public-http";
      const screenshot = await page.call("Page.captureScreenshot", { format: "png" });
      await writeFile(join(artifacts, `${name}.png`), Buffer.from(screenshot.data, "base64"));
      reports.push({ name, content, fonts });
    }
  }
  await page.evaluate("Reveal.slide(1,0,1)");
  await page.evaluate("Reveal.getPlugin('notes').open()", true);
  const popupTarget = await poll(async () => (await targets()).find(candidate => candidate.type === "page" && candidate.id !== target.id));
  const popup = await connect(popupTarget, ports);
  await poll(() => popup.evaluate("document.body?.textContent.includes('PRIVATE_NOTE')&&[...document.querySelectorAll('iframe')].length===2&&[...document.querySelectorAll('iframe')].every(frame=>frame.contentWindow.Reveal?.isReady())"));
  assert.deepEqual(await popup.evaluate("probeViolations"), []);
  if (helper) {
    assert.equal(await popup.evaluate("document.querySelector('.speaker-controls-notes .math-rendered') !== null"), true);
    const notesMath = await popup.evaluate(`(() => {
      const root = document.querySelector('.speaker-controls-notes .math-rendered');
      const source = root.querySelector('.math-source');
      const rectangle = root.querySelector('svg').getBoundingClientRect();
      const citation = document.querySelector('.speaker-controls-notes .citation');
      const declaration = selector => getComputedStyle(citation.querySelector(selector));
      return { source: getComputedStyle(source).display, tex: source.textContent,
        svgVisible: rectangle.width > 0 && rectangle.height > 0,
        smallcaps: declaration('.csl-smallcaps').fontVariant,
        normalEmphasis: declaration('.csl-normal-emphasis').fontStyle,
        normalStrong: declaration('.csl-normal-strong').fontWeight,
        normalSmallcaps: declaration('.csl-normal-smallcaps').fontVariant,
        linkSize: declaration('.citation-link').fontSize,
        citationSize: getComputedStyle(citation).fontSize };
    })()`);
    assert.equal(notesMath.source, "none");
    assert.equal(notesMath.tex, "\\begin{equation}n=4\\label{result}\\end{equation}");
    assert.equal(notesMath.svgVisible, true);
    assert.equal(notesMath.smallcaps, "small-caps");
    assert.equal(notesMath.normalEmphasis, "normal");
    assert.equal(notesMath.normalStrong, "400");
    assert.equal(notesMath.normalSmallcaps, "normal");
    assert.ok(Number.parseFloat(notesMath.linkSize) < Number.parseFloat(notesMath.citationSize));
    reports.push({ name: "speaker-math", styles: notesMath });
    const screenshot = await popup.call("Page.captureScreenshot", { format: "png" });
    await writeFile(join(artifacts, "presenter-notes.png"), Buffer.from(screenshot.data, "base64"));
    console.log(`research slides verified: static SVG/TeX, citations, footnotes, tables, CJK DOM/SVG; artifacts: ${artifacts}`);
  }
  // Live reload restores the Reveal hash; it does not retain a separate position model.
  const preview = child(binary, ["preview", "talk.adoc", "--to", "revealjs", "--port", "0", ...researchArguments]);
  const previewPort = await poll(() => { preview.check(); return preview.stderr().match(/http:\/\/127\.0\.0\.1:(\d+)\//)?.[1]; });
  ports.add(previewPort);
  const previewUrl = `http://127.0.0.1:${previewPort}/`;
  await page.call("Page.navigate", { url: previewUrl });
  await poll(() => page.evaluate(`location.href.split('#')[0]===${JSON.stringify(previewUrl)}&&probeReady`));
  const original = await readFile(join(root, "talk.adoc"), "utf8");
  const saveToDom = [];
  async function update(source, marker) {
    await writeFile(join(root, "talk.adoc"), `${source}\n\n${marker}\n`);
    const saved = performance.now();
    await poll(() => page.evaluate(`probeReady&&Reveal.isReady()&&document.body.textContent.includes(${JSON.stringify(marker)})`));
    return performance.now() - saved;
  }
  for (let index = 0; index < (helper ? 20 : 1); index += 1) {
    saveToDom.push(await update(original, `SAVE_MARKER_${index}`));
  }
  await page.evaluate("Reveal.slide(1,0,1)");
  await poll(() => page.evaluate("location.hash==='#/method/1'"));
  const oneFragment = original.replace(/^\* \[\[second-step\]\].*\n/m, "")
    .replaceAll("<<second-step>>", "second step");
  await update(oneFragment, "REMOVED_FRAGMENT");
  const fragmentFallback = await page.evaluate("Reveal.getIndices()");
  assert.deepEqual(fragmentFallback, { h: 1, v: 0, f: 0 });
  const removedHeading = oneFragment.replace("[#method]", "[#changed-method]").replaceAll("<<method>>", "method");
  await update(removedHeading, "REMOVED_HEADING");
  const headingFallback = await page.evaluate("Reveal.getIndices()");
  assert.deepEqual(headingFallback, { h: 0, v: 0 });
  assert.deepEqual(await page.evaluate("probeViolations"), []);
  if (helper) {
    const sorted = [...saveToDom].sort((left, right) => left - right);
    const live = { node: process.version, platform: process.platform, saveToDomMs: saveToDom,
      minimum: sorted[0], median: (sorted[9] + sorted[10]) / 2, maximum: sorted.at(-1), fragmentFallback, headingFallback };
    reports.push({ name: "live-preview", ...live });
    await writeFile(join(artifacts, "research.json"), `${JSON.stringify(reports, null, 2)}\n`);
    console.log(`preview save-close to DOM/Reveal ready: ${JSON.stringify(live)}`);
  }
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
