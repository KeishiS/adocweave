// Real fixed-runtime regression: file URLs, managed HTTP, fragments and notes.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { connectCdp } from "./browser-cdp.mjs";
import { hostExecutableEnvironment, resolveHostExecutable } from "./host-executable.mjs";

const [binaryArgument, chromiumArgument, helperArgument, artifactArgument] = process.argv.slice(2);
assert.ok(binaryArgument && chromiumArgument, "usage: node tools/slides-browser-smoke.mjs CLI CHROMIUM [HELPER_MJS [ARTIFACT_DIRECTORY]]");
const binary = resolve(binaryArgument);
const chromium = await resolveHostExecutable(chromiumArgument);
const helper = helperArgument && resolve(helperArgument);
const artifacts = helper && resolve(artifactArgument ?? "target/slides-browser");
const pdfInfo = helper && await resolveHostExecutable("pdfinfo");
const pdfText = helper && await resolveHostExecutable("pdftotext");
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
async function connect(target, ports, onlyFile) {
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  sockets.push(socket);
  const errors = [];
  const blocked = [];
  function allowedRequest(value) {
    const url = new URL(value);
    if (onlyFile) {
      url.search = "";
      url.hash = "";
      return url.href === onlyFile || url.protocol === "data:";
    }
    return url.protocol === "file:" || url.protocol === "about:" || (url.protocol === "http:" && url.hostname === "127.0.0.1" && ports.has(url.port));
  }
  const transport = await connectCdp(socket, { onEvent: async message => {
    if (message.method === "Runtime.exceptionThrown") errors.push(message.params.exceptionDetails);
    else if (message.method === "Log.entryAdded" && message.params.entry.level === "error") errors.push(message.params.entry);
    else if (onlyFile && message.method === "Network.requestWillBeSent" && !allowedRequest(message.params.request.url)) {
      blocked.push(message.params.request.url);
    }
    else if (message.method === "Fetch.requestPaused") {
      const { requestId, request } = message.params;
      const allowed = allowedRequest(request.url);
      if (!allowed) blocked.push(request.url);
      await transport.call(allowed ? "Fetch.continueRequest" : "Fetch.failRequest", allowed ? { requestId } : { requestId, errorReason: "BlockedByClient" });
    }
  }});
  const { call } = transport;
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
  return { call, evaluate, errors, blocked, check: transport.check };
}
function inspectPdf(tool, arguments_) {
  const result = spawnSync(tool, arguments_, { env: { ...environment, LC_ALL: "C" }, encoding: "utf8" });
  assert.equal(result.status, 0, result.error?.message ?? result.stderr);
  return result.stdout;
}
async function singleFile(source, bundle, arguments_ = []) {
  const conversion = spawnSync(binary, ["convert", source, "--to", "revealjs", "--single-file", ...arguments_],
    { cwd: root, env: environment, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  assert.equal(conversion.status, 0, conversion.error?.message ?? conversion.stderr);
  assert.doesNotMatch(conversion.stdout, /PRIVATE_NOTE|PRIVATE_LAST_NOTE|PRIVATE_BIBLIOGRAPHY/);
  // This directory contains only the distributed file, with no adjacent assets.
  const directory = await mkdtemp(join(root, "single-file-"));
  const path = join(directory, "配布 slides.html");
  await writeFile(path, conversion.stdout);
  assert.deepEqual(await readdir(directory), ["配布 slides.html"]);
  const manifest = JSON.parse(await readFile(join(root, bundle, ".adocweave-manifest.json"), "utf8"));
  const images = new Map();
  const notices = [];
  const mime = { Png: "image/png", Jpeg: "image/jpeg", Gif: "image/gif", Webp: "image/webp", Svg: "image/svg+xml" };
  for (const file of manifest.files) {
    if (file.path.startsWith("licenses/")) {
      notices.push([file.path, await readFile(join(root, bundle, file.path), "utf8")]);
    } else if (mime[file.mediaType]) {
      const bytes = await readFile(join(root, bundle, file.path));
      images.set(`data:${mime[file.mediaType]};base64,${bytes.toString("base64")}`, file.path);
    }
  }
  return { url: pathToFileURL(path).href, images, notices };
}
async function singleFileNotices(page, expected) {
  const content = await page.evaluate(`(() => {
    const licenses = document.querySelector('.slides-licenses');
    return {
      notices: [...licenses.querySelectorAll('section')].map(section => [section.querySelector('h2').textContent, section.querySelector('pre').textContent]),
      open: licenses.open,
      embeddedScripts: [...document.scripts].every(script => !script.hasAttribute('src')),
      embeddedStyles: [...document.querySelectorAll('link[rel="stylesheet"]')].every(link => link.getAttribute('href').startsWith('data:')),
      embeddedImages: [...document.images].every(image => image.getAttribute('src').startsWith('data:') && image.complete && image.naturalWidth > 0),
    };
  })()`);
  assert.deepEqual(content.notices.map(([path]) => path).sort(), expected.map(([path]) => path).sort(),
    "Single-file license inventory differs from the bundle");
  const notices = new Map(content.notices);
  for (const [path, text] of expected) {
    assert.ok(notices.get(path) === text, `Single-file license text differs from the bundle: ${path}`);
  }
  assert.equal(content.open, false);
  assert.equal(content.embeddedScripts, true);
  assert.equal(content.embeddedStyles, true);
  assert.equal(content.embeddedImages, true);
  await page.evaluate("document.querySelector('.slides-licenses summary').click()");
  assert.equal(await page.evaluate("document.querySelector('.slides-licenses').open"), true);
  await page.evaluate("document.querySelector('.slides-licenses summary').click()");
  const citations = expected.some(([path]) => path === "licenses/citations-license.txt");
  assert.equal(await page.evaluate("Boolean(document.querySelector('.slides-attribution a[data-slides-license]'))"), citations);
  if (citations) {
    const indices = await page.evaluate("Reveal.getIndices()");
    const license = await page.evaluate(`(() => {
      const link = document.querySelector('.slides-attribution a[data-slides-license]');
      link.click();
      const notice = document.getElementById(link.getAttribute('href').slice(1));
      const bounds = notice.getBoundingClientRect();
      const summary = notice.closest('details').querySelector('summary');
      const summaryBounds = summary.getBoundingClientRect();
      const summaryPoint = { x: summaryBounds.left + summaryBounds.width / 2, y: summaryBounds.top + summaryBounds.height / 2 };
      return { path: notice.closest('section').querySelector('h2').textContent,
        open: notice.closest('details').open, visible: bounds.width > 0 && bounds.bottom > 0 && bounds.top < innerHeight,
        summaryPoint, summaryVisible: summaryBounds.width > 0 && summaryBounds.height > 0 && summaryBounds.top >= 0 && summaryBounds.bottom <= innerHeight,
        summaryReachable: document.elementFromPoint(summaryPoint.x, summaryPoint.y)?.closest('summary') === summary };
    })()`);
    assert.equal(license.path, "licenses/citations-license.txt");
    assert.equal(license.open, true);
    assert.equal(license.visible, true);
    assert.equal(license.summaryVisible, true, "License close control scrolled out of the viewport");
    assert.equal(license.summaryReachable, true, "License close control is covered");
    assert.deepEqual(await page.evaluate("Reveal.getIndices()"), indices, "Opening a license changed the slide");
    for (const type of ["mousePressed", "mouseReleased"]) {
      await page.call("Input.dispatchMouseEvent", { type, ...license.summaryPoint, button: "left", clickCount: 1 });
    }
    assert.equal(await page.evaluate("document.querySelector('.slides-licenses').open"), false);
    assert.deepEqual(await page.evaluate("Reveal.getIndices()"), indices, "Closing a license changed the slide");
  }
  assert.deepEqual(await page.evaluate("probeViolations"), []);
}
async function screenLayout(page, ratio) {
  const layouts = await page.evaluate(`(() => {
    const layouts = [];
    for (const slideNode of Reveal.getSlides()) {
      const { h, v } = Reveal.getIndices(slideNode);
      Reveal.slide(h, v, Number.MAX_SAFE_INTEGER);
      const slide = Reveal.getCurrentSlide();
      const bounds = slide.getBoundingClientRect();
      const style = getComputedStyle(slide);
      const scale = Reveal.getScale();
      const content = {
        left: bounds.left + parseFloat(style.paddingLeft) * scale,
        right: bounds.right - parseFloat(style.paddingRight) * scale,
        top: bounds.top + parseFloat(style.paddingTop) * scale,
        bottom: bounds.bottom - parseFloat(style.paddingBottom) * scale,
      };
      const outside = [...slide.querySelectorAll('h1,h2,h3,p,li,figure,table,[data-math-display="block"]')]
        .filter(node => !node.closest('aside.notes'))
        .map(node => ({ tag: node.tagName, bounds: node.getBoundingClientRect() }))
        .filter(({ bounds: box }) => box.width > 0 && box.height > 0 &&
          (box.left < content.left - 1 || box.right > content.right + 1 ||
           box.top < content.top - 1 || box.bottom > content.bottom + 1))
        .map(({ tag }) => tag);
      const footnotes = slide.querySelector(':scope > .slide-body > .footnotes');
      const footnoteBounds = footnotes?.getBoundingClientRect();
      const bodyBounds = slide.querySelector(':scope > .slide-body > .slide-content').getBoundingClientRect();
      const inlineEquation = footnotes?.querySelector('[data-math-display="inline"] .math-rendered > svg')
        ?.getBoundingClientRect();
      layouts.push({ slide: slide.id, outside, alignment: style.textAlign,
        footnote: footnoteBounds && {
          bottomDifference: Math.abs(footnoteBounds.bottom - content.bottom),
          contentGap: footnoteBounds.top - bodyBounds.bottom,
          equationWidth: inlineEquation?.width,
          width: footnoteBounds.width,
        },
      });
    }
    Reveal.slide(1, 0, 1);
    return { aspect: Reveal.getConfig().width / Reveal.getConfig().height, slides: layouts };
  })()`);
  assert.ok(Math.abs(layouts.aspect - ratio) < 1e-8);
  for (const layout of layouts.slides) {
    assert.equal(layout.alignment, "left", layout.slide);
    assert.deepEqual(layout.outside, [], layout.slide);
    if (layout.footnote) {
      assert.ok(layout.footnote.bottomDifference <= 1, `Footnotes must align to the slide bottom: ${layout.slide}`);
      assert.ok(layout.footnote.contentGap >= 0, `Footnotes overlap slide content: ${layout.slide}`);
      assert.ok(layout.footnote.equationWidth > 0 && layout.footnote.equationWidth < layout.footnote.width / 2,
        `Inline footnote equation is missing or occupies the slide width: ${layout.slide}`);
    }
  }
}
async function navigation(page, url) {
  await page.call("Page.navigate", { url });
  await poll(() => page.evaluate(`location.href.split('#')[0]===${JSON.stringify(url)}&&probeReady`));
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
}
async function aspectRatios(page, researchArguments) {
  const research = await readFile(join(root, "research.adoc"), "utf8");
  const cases = [
    ["screen-wide", "", 1920, 1200, 16 / 10],
    ["screen-widescreen", ":slides-aspect-ratio: auto", 1920, 1080, 16 / 9],
    ["screen-standard", ":slides-aspect-ratio: auto", 1600, 1200, 4 / 3],
    ["screen-unset", ":slides-aspect-ratio!:", 1920, 1200, 16 / 10],
    ["fixed-widescreen", ":slides-aspect-ratio: 16:9", 1920, 1200, 16 / 9],
    ["fixed-standard", ":slides-aspect-ratio: 4:3", 1920, 1200, 4 / 3],
    ["screen-unavailable", "", 0, 0, 16 / 9],
    ["screen-negative", "", -1920, -1200, 16 / 9],
    ["screen-out-of-range", "", 6000, 1000, 16 / 9],
  ];
  for (const [name, attribute, width, height, ratio] of cases) {
    await writeFile(join(root, "aspect.adoc"), research.replace(/^:slides-aspect-ratio:.*$/m, attribute));
    const conversion = spawnSync(binary, ["convert", "aspect.adoc", "--to", "revealjs", "--output", name, ...researchArguments],
      { cwd: root, env: environment, encoding: "utf8" });
    assert.equal(conversion.status, 0, conversion.stderr);
    await page.call("Emulation.setDeviceMetricsOverride", { width: 800, height: 600, deviceScaleFactor: 1, mobile: false });
    const injection = await page.call("Page.addScriptToEvaluateOnNewDocument", { source:
      `Object.defineProperty(screen,'width',{get:()=>${width}});Object.defineProperty(screen,'height',{get:()=>${height}});` });
    const url = pathToFileURL(join(root, name, "index.html")).href;
    try {
      await page.call("Page.navigate", { url });
      await poll(() => page.evaluate(`location.href.split('#')[0] === ${JSON.stringify(url)} && probeReady && Reveal.isReady()`));
      const dimensions = await page.evaluate("({width:Reveal.getConfig().width,height:Reveal.getConfig().height,screen:[screen.width,screen.height],viewport:[innerWidth,innerHeight]})");
      assert.equal(dimensions.width, 1280, name);
      assert.ok(Math.abs(dimensions.width / dimensions.height - ratio) < 1e-8, name);
      assert.deepEqual(dimensions.screen, [width, height], name);
      assert.deepEqual(dimensions.viewport, [800, 600], name);
      await screenLayout(page, ratio);
      await page.call("Emulation.setDeviceMetricsOverride", { width: 1200, height: 800, deviceScaleFactor: 1, mobile: false });
      await page.evaluate("new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))");
      assert.deepEqual(await page.evaluate("[Reveal.getConfig().width,Reveal.getConfig().height]"),
        [dimensions.width, dimensions.height], `Resize changed aspect ratio: ${name}`);
      await printPdf(page, url, `aspect-${name}`, ratio);
      reports.push({ name: `aspect-${name}-screen`, dimensions, ratio });
    } finally {
      await page.call("Page.removeScriptToEvaluateOnNewDocument", { identifier: injection.identifier });
    }
  }
  await page.call("Emulation.clearDeviceMetricsOverride");
}
async function printPdf(page, url, name, ratio = 16 / 9) {
  const printUrl = `${url}?print-pdf`;
  await page.call("Page.navigate", { url: printUrl });
  await poll(() => page.evaluate(`location.href.split('#')[0] === ${JSON.stringify(printUrl)} &&
    probeReady && document.querySelectorAll('.pdf-page').length > 0`));
  // Preview errors belong to the interactive view, even when diagnostics exist.
  await page.evaluate(`(() => {
    const diagnostics = document.querySelector('.slides-diagnostics');
    if (diagnostics) { diagnostics.hidden = false; diagnostics.textContent = 'PRINT_DIAGNOSTIC_PROBE'; }
    const licenses = document.querySelector('.slides-licenses');
    if (licenses) licenses.open = true;
  })()`);
  // Resolve print styles and layout before capture, including non-16:9 pages.
  await page.call("Emulation.setEmulatedMedia", { media: "print" });
  const layout = await page.evaluate(`(() => {
    const rectangle = node => {
      const box = node.getBoundingClientRect();
      return { left: box.left, right: box.right, top: box.top, bottom: box.bottom };
    };
    const pages = [...document.querySelectorAll('.pdf-page')].map(page => {
      const slide = page.querySelector('section');
      const bounds = rectangle(page);
      const style = getComputedStyle(slide);
      const box = rectangle(slide);
      const content = { left: box.left + parseFloat(style.paddingLeft), right: box.right - parseFloat(style.paddingRight),
        top: box.top + parseFloat(style.paddingTop), bottom: box.bottom - parseFloat(style.paddingBottom) };
      const outside = [...slide.querySelectorAll('h1,h2,h3,p,li,figure,table,.math-rendered > svg')]
        .filter(node => !node.closest('aside.notes'))
        .filter(node => {
          const box = node.getBoundingClientRect();
          return box.width > 0 && box.height > 0 && (box.left < content.left - 1 || box.right > content.right + 1 ||
            box.top < content.top - 1 || box.bottom > content.bottom + 1);
        }).map(node => node.tagName);
      const footnotes = slide.querySelector(':scope > .slide-body > .footnotes');
      // Reveal shortens each PDF page wrapper by 1px to avoid extra blank pages.
      return { slide: slide.id, outside, withinPage: box.top >= bounds.top - 1 && box.bottom <= bounds.bottom + 1,
        fillsPageWidth: Math.abs(box.left - bounds.left) <= 1 && Math.abs(box.right - bounds.right) <= 1,
        footnotesAtBottom: !footnotes || Math.abs(rectangle(footnotes).bottom - content.bottom) <= 1 };
    });
    const credits = document.querySelector('.slides-attribution');
    const creditBox = rectangle(credits);
    const topmost = document.elementFromPoint(Math.min(creditBox.right - 4, innerWidth - 4), creditBox.bottom - 3);
    const hiddenFragments = [...document.querySelectorAll('.pdf-page .fragment')]
      .filter(node => getComputedStyle(node).visibility === 'hidden' || getComputedStyle(node).opacity === '0').length;
    const licenses = document.querySelector('.slides-licenses');
    return { pages, hiddenFragments, creditsVisible: topmost?.closest('.slides-attribution') === credits,
      licensesHidden: !licenses || getComputedStyle(licenses).display === 'none', violations: probeViolations };
  })()`);
  assert.equal(layout.pages.length, await page.evaluate("Reveal.getTotalSlides()"), name);
  assert.equal(await page.evaluate("Reveal.getConfig().pdfSeparateFragments"), false, name);
  assert.equal(layout.hiddenFragments, 0, `Print must show the final fragment state: ${name}`);
  assert.deepEqual(layout.violations, [], name);
  assert.equal(layout.creditsVisible, true, `Attribution is covered by PDF page backgrounds: ${name}`);
  assert.equal(layout.licensesHidden, true, `License viewer must not add PDF pages: ${name}`);
  for (const slide of layout.pages) {
    assert.equal(slide.withinPage, true, `${name}: ${slide.slide}`);
    assert.equal(slide.fillsPageWidth, true, `Unexpected PDF layout margins: ${name}: ${slide.slide}`);
    assert.deepEqual(slide.outside, [], `${name}: ${slide.slide}`);
    assert.equal(slide.footnotesAtBottom, true, `${name}: ${slide.slide}`);
  }
  const path = join(artifacts, `${name}.pdf`);
  const pdf = await page.call("Page.printToPDF", { printBackground: true, preferCSSPageSize: true,
    displayHeaderFooter: false, marginTop: 0, marginBottom: 0, marginLeft: 0, marginRight: 0 });
  await writeFile(path, Buffer.from(pdf.data, "base64"));
  const info = inspectPdf(pdfInfo, [path]);
  assert.equal(Number(info.match(/^Pages:\s+(\d+)/m)?.[1]), layout.pages.length, `Missing PDF pages: ${name}`);
  const size = info.match(/^Page size:\s+([\d.]+) x ([\d.]+) pts/m);
  assert.ok(size && Math.abs(Number(size[1]) - 960) < 1 && Math.abs(Number(size[2]) - 960 / ratio) < 1,
    `Unexpected PDF dimensions or outer margins: ${name}: ${info}`);
  const text = inspectPdf(pdfText, ["-enc", "UTF-8", path, "-"]);
  const compact = text.replace(/\s+/gu, "");
  for (const content of ["研究スライドの受入原稿", "Method", "Vertical", "Detail", "Last", "References",
    "観測値の取得", "結果の推定", "補助条件として", "を仮定します", "出典", "Doe", "Roe", "FrankBennett"]) {
    assert.ok(compact.includes(content), `Missing printed content (${content}): ${name}`);
  }
  assert.doesNotMatch(text, /PRIVATE_NOTE|PRIVATE_LAST_NOTE|PRIVATE_BIBLIOGRAPHY|PRINT_DIAGNOSTIC_PROBE/);
  await writeFile(join(artifacts, `${name}.txt`), text);
  reports.push({ name, ...layout, info });
  await page.call("Emulation.setEmulatedMedia", { media: "screen" });
}
async function overflowPdf(page) {
  const markers = Array.from({ length: 60 }, (_, index) => `OVERFLOW_PARAGRAPH_${index + 1}_END`);
  await writeFile(join(root, "overflow.adoc"), "= Overflow\n:slides-aspect-ratio: 16:9\n\n== Long slide\n\n" +
    markers.join("\n\n") + "\n\n[%step]\n* FINAL_FRAGMENT_MARKER\n\n[.notes]\n--\nPRIVATE_OVERFLOW_NOTE\n--\n");
  const conversion = spawnSync(binary, ["convert", "overflow.adoc", "--to", "revealjs", "--output", "overflow", "--audience", "presenter"],
    { cwd: root, env: environment, encoding: "utf8" });
  assert.equal(conversion.status, 0, conversion.stderr);
  const url = `${pathToFileURL(join(root, "overflow", "index.html")).href}?print-pdf`;
  await page.call("Page.navigate", { url });
  await poll(() => page.evaluate(`location.href.split('#')[0] === ${JSON.stringify(url)} &&
    probeReady && document.querySelectorAll('.pdf-page').length === 2`));
  await page.call("Emulation.setEmulatedMedia", { media: "print" });
  const path = join(artifacts, "overflow-print.pdf");
  const pdf = await page.call("Page.printToPDF", { printBackground: true, preferCSSPageSize: true,
    displayHeaderFooter: false, marginTop: 0, marginBottom: 0, marginLeft: 0, marginRight: 0 });
  await writeFile(path, Buffer.from(pdf.data, "base64"));
  const info = inspectPdf(pdfInfo, [path]);
  const pages = Number(info.match(/^Pages:\s+(\d+)/m)?.[1]);
  assert.ok(pages > 2, `Long slide must continue across PDF pages: ${info}`);
  const text = inspectPdf(pdfText, ["-enc", "UTF-8", path, "-"]);
  for (const marker of [...markers, "FINAL_FRAGMENT_MARKER"]) {
    assert.equal(text.split(marker).length - 1, 1, `Overflow content missing or duplicated: ${marker}`);
  }
  assert.doesNotMatch(text, /PRIVATE_OVERFLOW_NOTE/);
  assert.deepEqual(await page.evaluate("probeViolations"), []);
  await writeFile(join(artifacts, "overflow-print.txt"), text);
  reports.push({ name: "overflow-print", pages, info });
  await page.call("Emulation.setEmulatedMedia", { media: "screen" });
}
async function researchDisplay(page, url, presenter, embeddedImages = new Map(), reportName) {
  await page.call("Page.navigate", { url });
  await poll(() => page.evaluate(`location.href.split('#')[0]===${JSON.stringify(url)}&&probeReady`));
  assert.equal(await page.evaluate("Reveal.getTotalSlides()"), helper ? 6 : 5);
  assert.deepEqual(await page.evaluate("probeViolations"), []);
  assert.equal(await page.evaluate("document.querySelectorAll('iframe').length"), 0);
  if (helper) {
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
        sourcesHidden: math.every(node => node.querySelector('.math-source')?.hidden),
        matrix: document.querySelectorAll('#vertical svg [data-mml-node="mtable"] [data-mml-node="mtr"]').length,
        tableMath: document.querySelectorAll('#detail td .math-rendered').length,
        tableCitation: document.querySelectorAll('#detail td .citation').length,
        duplicateIds: ids.length - new Set(ids).size, missing,
        footnote: label('#method > .slide-body > .footnotes li .math-rendered'),
        notesFootnote: label('#method aside.notes .footnotes li .math-rendered'),
        footnoteMath: [...document.querySelectorAll('.footnotes [data-math-display]')].map(node => ({
          display: node.dataset.mathDisplay,
          source: node.querySelector('.math-source')?.textContent,
          numbered: Boolean(node.querySelector('[data-mml-node="mlabeledtr"]')),
          glyphs: [...node.querySelectorAll('.math-rendered svg use[data-c]')].map(glyph => glyph.getAttribute('data-c')),
        })),
        notesResult: label(notesMath.find(node => node.closest('#method') &&
          node.querySelector('.math-source')?.textContent.includes('label{result}'))),
        citation: text('#method p .citation'),
        bodyReferences: [...document.querySelectorAll('.bibliography-anchor[id^="slides-body-bib-"]')].map(node => node.id),
        notesReferences: [...document.querySelectorAll('.bibliography-anchor[id^="slides-notes-bib-"]')].map(node => node.id),
        privateText: document.body.textContent.includes('PRIVATE_BIBLIOGRAPHY'),
        imageReady: [...document.images].every(image => image.complete && image.naturalWidth > 0),
        images: [...document.querySelectorAll('.reveal img')].filter(image => !image.closest('aside.notes'))
          .map(image => [...image.attributes].map(attribute => [attribute.name, attribute.value])),
        accent: getComputedStyle(document.querySelector('.reveal .controls')).color,
        creditsVisible: rectangle.width > 0 && rectangle.height > 0 && rectangle.bottom <= innerHeight + 1,
        credits: credits.textContent,
        browserMathJax: [...document.scripts].some(script => /mathjax|citeproc/i.test(script.src)),
        body: [...document.querySelectorAll('.math-rendered, .citation, .footnotes li, #slides-body-references li, figcaption')]
          .filter(node => !node.closest('aside.notes'))
          .map(node => [node.className, node.textContent.replace(/\\s+/g, ' ').trim(), node.querySelector('svg')?.outerHTML ?? '']),
      };
    })()`);
    assert.equal(content.bodyMath, 8);
    assert.equal(content.notesMath, presenter ? 2 : 0);
    assert.equal(content.svg, presenter ? 10 : 8);
    assert.equal(content.sourcesHidden, true);
    assert.equal(content.matrix, 2);
    assert.equal(content.tableMath, 1);
    assert.equal(content.tableCitation, 1);
    assert.equal(content.duplicateIds, 0);
    assert.deepEqual(content.missing, []);
    assert.equal(content.footnote, "");
    assert.deepEqual(content.footnoteMath,
      Array.from({ length: 2 }, () => ({ display: "inline", source: "z=3", numbered: false,
        glyphs: ["1D467", "3D", "33"] })));
    assert.equal(content.citation, "(Doe2024b,pp.12–15)↗");
    assert.equal(content.bodyReferences.length, 3);
    assert.equal(content.notesReferences.length, presenter ? 1 : 0);
    assert.equal(content.privateText, presenter);
    if (presenter) {
      assert.equal(content.notesFootnote, "");
      assert.equal(content.notesResult, "(1)");
    }
    assert.equal(content.imageReady, true);
    assert.equal(content.accent, "rgb(15, 118, 110)");
    await screenLayout(page, 16 / 9);
    assert.equal(content.creditsVisible, true);
    assert.match(content.credits, /Frank Bennett/);
    assert.match(content.credits, /citeproc-js implements the Citation Style Language/);
    assert.ok(content.credits.includes("https://citationstyles.org/"));
    assert.equal(content.browserMathJax, false);
    const images = content.images.map(attributes => attributes.map(([name, value]) => {
      if (name !== "src" || !value.startsWith("data:")) return [name, value];
      assert.ok(embeddedImages.has(value), "Embedded image differs from the verified bundle asset");
      return [name, embeddedImages.get(value)];
    }));
    const body = { content: content.body, images };
    if (publicBody) assert.deepEqual(body, publicBody);
    else publicBody = body;
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
      for (const [kind, selector] of [
        ["heading", "#method h2"],
        ["body", "#method .column p"],
        ["caption", "#method figcaption"],
        ["svg", "#method .math-rendered svg text"],
        ["footnote", "#method > .slide-body > .footnotes li"],
      ]) {
        const { nodeId } = await page.call("DOM.querySelector", { nodeId: document_.nodeId, selector });
        assert.ok(nodeId, `Missing CJK ${kind} probe node.`);
        fonts[kind] = (await page.call("CSS.getPlatformFontsForNode", { nodeId })).fonts;
        assert.ok(fonts[kind].some(font => /Noto.*Serif.*CJK/.test(font.familyName) && font.glyphCount >= 2 && !font.isCustomFont),
          `Missing system Mincho glyphs in ${kind}: ${JSON.stringify(fonts[kind])}`);
      }
    }
    const name = reportName ?? (url.startsWith("file:") ? "public-file" : presenter ? "presenter-http" : "public-http");
    const screenshot = await page.call("Page.captureScreenshot", { format: "png" });
    await writeFile(join(artifacts, `${name}.png`), Buffer.from(screenshot.data, "base64"));
    reports.push({ name, content, fonts });
  }
}
async function defaultCitationStyle(page) {
  const conversion = spawnSync(binary, ["convert", "research.adoc", "--to", "revealjs", "--output", "defaults",
    "--slides-helper", helper, "--bibliography", "references.json"],
    { cwd: root, env: environment, encoding: "utf8" });
  assert.equal(conversion.status, 0, conversion.stderr);
  const url = pathToFileURL(join(root, "defaults", "index.html")).href;
  await page.call("Page.navigate", { url });
  await poll(() => page.evaluate(`location.href.split('#')[0]===${JSON.stringify(url)}&&probeReady`));
  const content = await page.evaluate(`(() => {
    const citationText = selector => {
      const citation = document.querySelector(selector).cloneNode(true);
      citation.querySelectorAll('.citation-link').forEach(node => node.remove());
      return citation.textContent.trim();
    };
    const references = document.getElementById('slides-body-references');
    const { h, v } = Reveal.getIndices(references);
    Reveal.slide(h, v);
    const credits = document.querySelector('.slides-attribution');
    const bounds = credits.getBoundingClientRect();
    return {
      firstCitation: citationText('#method p .citation'),
      citation: citationText('#detail td .citation'),
      referencesHeading: references.querySelector('h2').textContent,
      references: [...references.querySelectorAll('li')].map(node => node.textContent),
      referencesVisible: references.getBoundingClientRect().width > 0 && Reveal.getCurrentSlide() === references,
      privateText: document.body.textContent.includes('PRIVATE_BIBLIOGRAPHY'),
      accent: getComputedStyle(document.querySelector('.reveal .controls')).color,
      stylesheets: [...document.querySelectorAll('link[rel="stylesheet"]')].map(node => node.getAttribute('href')),
      credits: credits.textContent,
      creditsVisible: bounds.width > 0 && bounds.height > 0 && bounds.bottom <= innerHeight + 1,
      violations: probeViolations,
    };
  })()`);
  // The table cites the third source, following the main citation and its footnote.
  assert.equal(content.citation, "[3]");
  assert.match(content.firstCitation, /^\[1\b/);
  assert.match(content.firstCitation, /12[–-]15/);
  assert.equal(content.referencesHeading, "References");
  assert.equal(content.references.length, 3);
  for (const title of ["Zebra public result", "Inside footnote result", "Alpha public result"]) {
    assert.ok(content.references.some(reference => reference.includes(title)), title);
  }
  assert.equal(content.referencesVisible, true);
  assert.equal(content.privateText, false);
  assert.equal(content.accent, "rgb(15, 118, 110)");
  assert.deepEqual(content.stylesheets, ["assets/reset.css", "assets/reveal.css", "assets/theme.css", "assets/content.css"]);
  assert.match(content.credits, /Frank Bennett/);
  assert.match(content.credits, /citeproc-js implements the Citation Style Language/);
  assert.ok(content.credits.includes("https://citationstyles.org/"));
  assert.equal(content.creditsVisible, true);
  assert.deepEqual(content.violations, []);
  await screenLayout(page, 16 / 9);
  const screenshot = await page.call("Page.captureScreenshot", { format: "png" });
  await writeFile(join(artifacts, "default-citation-style.png"), Buffer.from(screenshot.data, "base64"));
  reports.push({ name: "default-citation-style", content });
}
async function speakerNotes(page, ports, targets, connections, url) {
  await page.call("Emulation.setDeviceMetricsOverride", {
    width: 1200, height: 800, screenWidth: 1920, screenHeight: 1200, deviceScaleFactor: 1, mobile: false,
  });
  await page.call("Page.navigate", { url });
  await poll(() => page.evaluate(`location.href.split('#')[0]===${JSON.stringify(url)}&&probeReady`));
  const dimensions = await page.evaluate("[Reveal.getConfig().width, Reveal.getConfig().height]");
  await page.evaluate("Reveal.slide(1,0,1)");
  const existingTargets = new Set((await targets()).map(target => target.id));
  await page.evaluate("Reveal.getPlugin('notes').open()", true);
  const popupTarget = await poll(async () => (await targets()).find(candidate => candidate.type === "page" && !existingTargets.has(candidate.id)));
  const popup = await connect(popupTarget, ports);
  connections.push(popup);
  await poll(() => popup.evaluate("document.body?.textContent.includes('PRIVATE_NOTE')&&[...document.querySelectorAll('iframe')].length===2&&[...document.querySelectorAll('iframe')].every(frame=>frame.contentWindow.Reveal?.isReady())"));
  // Moving the speaker window must not choose a different canvas on iframe reload.
  await popup.call("Emulation.setDeviceMetricsOverride", {
    width: 1200, height: 800, screenWidth: 1600, screenHeight: 1200, deviceScaleFactor: 1, mobile: false,
  });
  await popup.evaluate("[...document.querySelectorAll('iframe')].forEach(frame => { frame.contentWindow.reloadMarker = true; frame.contentWindow.location.reload(); })");
  await poll(() => popup.evaluate("[...document.querySelectorAll('iframe')].every(frame=>!frame.contentWindow.reloadMarker&&frame.contentWindow.Reveal?.isReady())"));
  const previews = await popup.evaluate(`Array.from(document.querySelectorAll('iframe'), frame => {
    const view = frame.contentWindow;
    const config = view.Reveal.getConfig();
    const box = view.document.querySelector('.reveal .slides').getBoundingClientRect();
    return { dimensions: [config.width, config.height], screen: [view.screen.width, view.screen.height], ratio: box.width / box.height };
  })`);
  for (const preview of previews) {
    assert.deepEqual(preview.screen, [1600, 1200]);
    assert.deepEqual(preview.dimensions, dimensions, "Speaker previews changed the presenting deck dimensions");
    assert.ok(Math.abs(preview.ratio - dimensions[0] / dimensions[1]) < 1e-6, "Speaker preview layout changed aspect ratio");
  }
  assert.deepEqual(await page.evaluate("[Reveal.getConfig().width, Reveal.getConfig().height]"), dimensions);
  reports.push({ name: "speaker-dimensions", dimensions, previews });
  assert.deepEqual(await popup.evaluate("probeViolations"), []);
  if (helper) {
    // Notes arrive through innerHTML; iframe readiness does not wait for their CSS.
    await poll(() => popup.evaluate("Boolean(document.querySelector('.speaker-controls-notes link[rel=\"stylesheet\"][href=\"assets/content.css\"]')?.sheet)"));
    assert.equal(await popup.evaluate("document.querySelector('.speaker-controls-notes .math-rendered') !== null"), true);
    const notesMath = await popup.evaluate(`(() => {
      const root = document.querySelector('.speaker-controls-notes .math-rendered');
      const source = root.querySelector('.math-source');
      const rectangle = root.querySelector('svg').getBoundingClientRect();
      const citation = document.querySelector('.speaker-controls-notes .citation');
      const declaration = selector => getComputedStyle(citation.querySelector(selector));
      return { source: getComputedStyle(source).display, sourceHidden: source.hidden, tex: source.textContent,
        svgVisible: rectangle.width > 0 && rectangle.height > 0,
        smallcaps: declaration('.csl-smallcaps').fontVariant,
        normalEmphasis: declaration('.csl-normal-emphasis').fontStyle,
        normalStrong: declaration('.csl-normal-strong').fontWeight,
        normalSmallcaps: declaration('.csl-normal-smallcaps').fontVariant,
        linkSize: declaration('.citation-link').fontSize,
        citationSize: getComputedStyle(citation).fontSize };
    })()`);
    assert.equal(notesMath.source, "none");
    assert.equal(notesMath.sourceHidden, true);
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
}
async function mathExtensionLayout(page, name) {
  const layout = await page.evaluate(`(() => {
    const slide = document.querySelector('#extensions');
    const box = node => { const r = node.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height }; };
    const visible = node => { const r = node.getBoundingClientRect(), style = getComputedStyle(node);
      return r.width > 0 && r.height > 0 && style.visibility === 'visible' && style.display !== 'none' && style.opacity !== '0'; };
    return {
      equations: [...slide.querySelectorAll('.math-rendered > svg')].map(node => ({ visible: visible(node), box: box(node) })),
      backgrounds: [...slide.querySelectorAll('rect[fill="pink"],rect[fill="lightblue"]')]
        .map(node => ({ fill: getComputedStyle(node).fill, visible: visible(node), box: box(node) })),
      cancellations: [...slide.querySelectorAll('[data-mml-node="menclose"] > line, [data-mml-node="menclose"] > path')].map(node => ({ visible: visible(node), box: box(node) })),
      violations: probeViolations,
    };
  })()`);
  assert.equal(layout.equations.length, 2, name);
  assert.ok(layout.equations.every(item => item.visible), name);
  assert.deepEqual(layout.backgrounds.map(item => item.fill).sort(), ["rgb(173, 216, 230)", "rgb(255, 192, 203)"], name);
  assert.ok(layout.backgrounds.every(item => item.visible), name);
  assert.ok(layout.cancellations.length > 0 && layout.cancellations.every(item => item.visible), name);
  assert.deepEqual(layout.violations, [], name);
  reports.push({ name, ...layout });
}
async function mathExtensionsDisplay(page, url, name) {
  for (const print of [false, true]) {
    const address = `${url}${print ? '?print-pdf' : ''}`;
    await page.call("Page.navigate", { url: address });
    await poll(() => page.evaluate(`location.href.split('#')[0]===${JSON.stringify(address)}&&probeReady&&Reveal.isReady()`));
    if (print) {
      await poll(() => page.evaluate("document.querySelectorAll('.pdf-page').length===2"));
      await page.call("Emulation.setEmulatedMedia", { media: "print" });
    } else {
      await page.evaluate("Reveal.slide(1)");
    }
    await mathExtensionLayout(page, `${name}-${print ? 'print' : 'screen'}`);
    if (!print) {
      const screenshot = await page.call("Page.captureScreenshot", { format: "png" });
      await writeFile(join(artifacts, `${name}.png`), Buffer.from(screenshot.data, "base64"));
    }
    if (print) {
      const pdf = await page.call("Page.printToPDF", { printBackground: true, preferCSSPageSize: true });
      const path = join(artifacts, `${name}.pdf`);
      await writeFile(path, Buffer.from(pdf.data, "base64"));
      assert.equal(Number(inspectPdf(pdfInfo, [path]).match(/^Pages:\s+(\d+)/m)?.[1]), 2, name);
      await page.call("Emulation.setEmulatedMedia", { media: "screen" });
    }
  }
}
async function mathExtensionsPreview(page, ports, config) {
  await writeFile(join(root, "math-extensions", ".adocweave.toml"), "schema-version = 2\n[math]\nextensions = []\n");
  const preview = child(binary, ["preview", "math-extensions/talk.adoc", "--to", "revealjs", "--port", "0", "--slides-helper", helper]);
  const port = await poll(() => { preview.check(); return preview.stderr().match(/http:\/\/127\.0\.0\.1:(\d+)\//)?.[1]; });
  ports.add(port);
  const url = `http://127.0.0.1:${port}/`;
  await page.call("Page.navigate", { url });
  await poll(() => page.evaluate("document.querySelector('.slides-diagnostics')?.textContent.includes('invalid-tex')"));
  assert.equal(await page.evaluate("document.querySelectorAll('.math-rendered > svg').length"), 0);
  // Only configuration changes; the manuscript and helper remain identical.
  await writeFile(join(root, "math-extensions", ".adocweave.toml"), config);
  await poll(() => page.evaluate("probeReady&&window.Reveal?.isReady()&&document.querySelectorAll('.math-rendered > svg').length===2"));
  await page.evaluate("Reveal.slide(1)");
  await mathExtensionLayout(page, "math-extensions-preview-recovery");
  assert.equal(await page.evaluate("document.querySelector('.slides-diagnostics')?.textContent.trim() ?? ''"), "");
}

async function livePreview(page, ports) {
  // Live reload restores the Reveal hash; it does not retain a separate position model.
  const preview = child(binary, ["preview", "navigation.adoc", "--to", "revealjs", "--port", "0"]);
  const previewPort = await poll(() => { preview.check(); return preview.stderr().match(/http:\/\/127\.0\.0\.1:(\d+)\//)?.[1]; });
  ports.add(previewPort);
  const previewUrl = `http://127.0.0.1:${previewPort}/`;
  await page.call("Page.navigate", { url: previewUrl });
  await poll(() => page.evaluate(`location.href.split('#')[0]===${JSON.stringify(previewUrl)}&&probeReady`));
  const original = await readFile(join(root, "navigation.adoc"), "utf8");
  const saveToDom = [];
  async function update(source, marker) {
    await writeFile(join(root, "navigation.adoc"), `${source}\n\n${marker}\n`);
    const saved = performance.now();
    await poll(() => page.evaluate(`probeReady&&Reveal.isReady()&&document.body.textContent.includes(${JSON.stringify(marker)})`));
    return performance.now() - saved;
  }
  for (let index = 0; index < 2; index += 1) {
    saveToDom.push(await update(original, `SAVE_MARKER_${index}`));
  }
  await page.evaluate("Reveal.slide(1,0,1)");
  await poll(() => page.evaluate("location.hash==='#/method/1'"));
  const oneFragment = original.replace(/^\* \[\[second-step\]\].*\n/m, "")
    .replaceAll("<<second-step>>", "second step");
  await update(oneFragment, "REMOVED_FRAGMENT");
  const fragmentFallback = await page.evaluate("Reveal.getIndices()");
  assert.deepEqual(fragmentFallback, { h: 1, v: 0, f: 0 });
  const removedHeading = oneFragment.replace(/^\[#method(?=[.\]])/m, "[#changed-method").replace(/<<method(?:,[^>]*)?>>/g, "method");
  await update(removedHeading, "REMOVED_HEADING");
  const headingFallback = await page.evaluate("Reveal.getIndices()");
  assert.deepEqual(headingFallback, { h: 0, v: 0 });
  assert.deepEqual(await page.evaluate("probeViolations"), []);
  if (helper) {
    const sorted = [...saveToDom].sort((left, right) => left - right);
    const live = { node: process.version, platform: process.platform, saveToDomMs: saveToDom,
      minimum: sorted[0], median: (sorted[0] + sorted[1]) / 2, maximum: sorted.at(-1), fragmentFallback, headingFallback };
    reports.push({ name: "live-preview", ...live });
    console.log(`preview save-close to DOM/Reveal ready: ${JSON.stringify(live)}`);
  }
}
try {
  await writeFile(join(root, "navigation.adoc"),
    await readFile(new URL("../fixtures/slides-browser/navigation.adoc", import.meta.url)));

  const researchArguments = [];
  if (helper) {
    for (const file of ["research.adoc", "references.json", "result.svg", "research.csl"]) {
      await writeFile(join(root, file),
        await readFile(new URL(`../fixtures/slides-browser/${file}`, import.meta.url)));
    }
    for (const [source, destination] of [["locale-en-US.xml", "locale.xml"]]) {
      await writeFile(join(root, destination), await readFile(new URL(`../packages/slides-helper/fixtures/${source}`, import.meta.url)));
    }
    researchArguments.push("--slides-helper", helper, "--bibliography", "references.json", "--csl-style", "research.csl",
      "--csl-locale", "locale.xml");
    await mkdir(artifacts, { recursive: true });
  }
  for (const audience of ["public", "presenter"]) {
    const started = performance.now();
    const conversion = spawnSync(binary, ["convert", helper ? "research.adoc" : "navigation.adoc", "--to", "revealjs", "--output", audience, "--audience", audience,
      ...researchArguments], { cwd: root, env: environment, encoding: "utf8" });
    assert.equal(conversion.status, 0, conversion.stderr);
    if (helper) reports.push({ name: `convert-${audience}`, milliseconds: performance.now() - started });
    const manifest = JSON.parse(await readFile(join(root, audience, ".adocweave-manifest.json"), "utf8"));
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
  const targets = async () => (await fetch(`http://127.0.0.1:${debuggingPort}/json/list`, { signal: AbortSignal.timeout(20_000) })).json();
  const failures = [];
  async function runCase(name, callback, onlyFile) {
    const previousTargets = new Set((await targets()).map(target => target.id));
    const connections = [];
    try {
      const target = await fetch(`http://127.0.0.1:${debuggingPort}/json/new?about:blank`, { method: "PUT", signal: AbortSignal.timeout(20_000) }).then(response => response.json());
      const page = await connect(target, ports, onlyFile);
      connections.push(page);
      await callback(page, connections);
      for (const connection of connections) {
        connection.check();
        assert.deepEqual(connection.blocked, [], name);
        assert.deepEqual(connection.errors.filter(error => !(error.source === "network" && error.url?.endsWith("/favicon.ico") && error.text?.includes("404"))), [], name);
      }
      console.log(`PASS ${name}`);
    } catch (error) {
      failures.push(new Error(name, { cause: error }));
      console.error(`FAIL ${name}: ${error.stack}`);
    } finally {
      for (const target of await targets()) {
        if (!previousTargets.has(target.id)) {
          await fetch(`http://127.0.0.1:${debuggingPort}/json/close/${target.id}`, { signal: AbortSignal.timeout(20_000) });
        }
      }
    }
  }
  const conversion = spawnSync(binary, ["convert", "navigation.adoc", "--to", "revealjs", "--output", "navigation"],
    { cwd: root, env: environment, encoding: "utf8" });
  assert.equal(conversion.status, 0, conversion.stderr);
  const navigationServer = await server("navigation");
  ports.add(new URL(navigationServer).port);
  for (const [name, url] of [["file", pathToFileURL(join(root, "navigation", "index.html")).href], ["http", `${navigationServer}/`]]) {
    await runCase(`navigation-${name}`, page => navigation(page, url));
  }
  const singleNavigation = await singleFile("navigation.adoc", "navigation");
  await runCase("navigation-single-file", async page => {
    await navigation(page, singleNavigation.url);
    await singleFileNotices(page, singleNavigation.notices);
  }, singleNavigation.url);
  for (const [name, url, presenter] of [["public-file", pathToFileURL(join(root, "public", "index.html")).href, false],
    ["public-http", `${publicServer}/`, false], ["presenter-http", `${presenterServer}/`, true]]) {
    await runCase(`research-${name}`, page => researchDisplay(page, url, presenter));
  }
  if (helper) {
    const singleResearch = await singleFile("research.adoc", "public", researchArguments);
    await runCase("research-single-file", async page => {
      await researchDisplay(page, singleResearch.url, false, singleResearch.images, "public-single-file");
      await singleFileNotices(page, singleResearch.notices);
      await printPdf(page, singleResearch.url, "public-single-file-print");
    }, singleResearch.url);
  }
  await runCase("speaker-notes", (page, connections) => speakerNotes(page, ports, targets, connections, `${presenterServer}/`));
  const autoSource = await readFile(join(root, helper ? "research.adoc" : "navigation.adoc"), "utf8");
  await writeFile(join(root, "presenter-auto.adoc"), autoSource.replace(/^:slides-aspect-ratio:.*$/m, ""));
  const autoConversion = spawnSync(binary, ["convert", "presenter-auto.adoc", "--to", "revealjs", "--output", "presenter-auto", "--audience", "presenter", ...researchArguments],
    { cwd: root, env: environment, encoding: "utf8" });
  assert.equal(autoConversion.status, 0, autoConversion.stderr);
  const autoServer = await server("presenter-auto");
  ports.add(new URL(autoServer).port);
  await runCase("speaker-notes-auto", (page, connections) => speakerNotes(page, ports, targets, connections, `${autoServer}/`));
  if (helper) {
    const extensionRoot = join(root, "math-extensions");
    await mkdir(extensionRoot);
    const extensionConfig = await readFile(new URL("../fixtures/slides-browser/math-extensions.toml", import.meta.url), "utf8");
    await writeFile(join(extensionRoot, ".adocweave.toml"), extensionConfig);
    await writeFile(join(extensionRoot, "talk.adoc"), await readFile(new URL("../fixtures/slides-browser/math-extensions.adoc", import.meta.url)));
    const extensions = spawnSync(binary, ["convert", "math-extensions/talk.adoc", "--to", "revealjs", "--output", "math-extensions/bundle", "--slides-helper", helper],
      { cwd: root, env: environment, encoding: "utf8" });
    assert.equal(extensions.status, 0, extensions.stderr);
    await runCase("math-extensions-file", page => mathExtensionsDisplay(page, pathToFileURL(join(extensionRoot, "bundle", "index.html")).href, "math-extensions-file"));
    const singleExtensions = await singleFile("math-extensions/talk.adoc", "math-extensions/bundle", ["--slides-helper", helper]);
    await runCase("math-extensions-single-file", page => mathExtensionsDisplay(page, singleExtensions.url, "math-extensions-single-file"), singleExtensions.url);
    await runCase("math-extensions-preview", page => mathExtensionsPreview(page, ports, extensionConfig));
    await runCase("default-citation-style", page => defaultCitationStyle(page));
    await runCase("pdf-file", page => printPdf(page, pathToFileURL(join(root, "public", "index.html")).href, "public-file-print"));
    await runCase("pdf-http", page => printPdf(page, `${publicServer}/`, "public-http-print"));
    await runCase("pdf-preview", async page => {
      const preview = child(binary, ["preview", "research.adoc", "--to", "revealjs", "--port", "0", ...researchArguments]);
      const port = await poll(() => { preview.check(); return preview.stderr().match(/http:\/\/127\.0\.0\.1:(\d+)\//)?.[1]; });
      ports.add(port);
      await printPdf(page, `http://127.0.0.1:${port}/`, "preview-print");
    });
    await runCase("aspect-ratios", page => aspectRatios(page, researchArguments));
    await runCase("pdf-overflow", page => overflowPdf(page));
  }
  await runCase("live-preview", page => livePreview(page, ports));
  if (helper) await writeFile(join(artifacts, "research.json"), `${JSON.stringify(reports, null, 2)}\n`);
  if (failures.length) throw new AggregateError(failures, "Slides browser cases failed");
  console.log("slides browser smoke passed: single file/managed HTTP, target fragment stages, reload, notes, CSP, offline");
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
