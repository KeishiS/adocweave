import { AdocWeaveClient, defaultAssetUrls } from "@adocweave/wasm";

const source = document.querySelector("#source");
const preview = document.querySelector("#preview");
const status = document.querySelector("#status");
let sourceRevision = 0;
let debounce;
let active;

const client = new AdocWeaveClient({
  ...defaultAssetUrls(new URL("../worker/index.mjs", import.meta.url)),
});

async function analyze(revision, text, controller) {
  try {
    const result = await client.analyze(
      { source: { text }, products: { html: true } },
      { signal: controller.signal },
    );
    if (revision !== sourceRevision) return;
    preview.textContent = result.html;
    status.value = `ready:${revision}`;
    globalThis.adocweaveLastResult = result;
  } catch (error) {
    if (controller.signal.aborted || revision !== sourceRevision) return;
    status.value = `error:${error.code ?? error.name}`;
  }
}

function update() {
  const revision = ++sourceRevision;
  const text = source.value;
  active?.abort();
  const controller = new AbortController();
  active = controller;
  clearTimeout(debounce);
  debounce = setTimeout(() => analyze(revision, text, controller), 40);
}

source.addEventListener("input", update);

if (new URL(location.href).searchParams.has("smoke")) {
  ++sourceRevision;
  const cancelled = new AbortController();
  const stale = client.analyze(
    { source: { text: "= stale result\n" }, products: { html: true } },
    { signal: cancelled.signal },
  );
  cancelled.abort();
  let abortRejected = false;
  try {
    await stale;
  } catch (error) {
    if (!cancelled.signal.aborted) throw error;
    abortRejected = true;
  }
  if (!abortRejected) throw new Error("aborted analysis unexpectedly completed");
  globalThis.adocweaveAbortSettled = true;
  source.value = "= Latest browser result\n";
  const revision = ++sourceRevision;
  active = new AbortController();
  await analyze(revision, source.value, active);
} else {
  update();
}
