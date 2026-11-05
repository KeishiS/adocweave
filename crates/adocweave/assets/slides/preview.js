/* Live preview instrumentation, separate from static and served slide bundles. */
(() => {
  "use strict";
  // Speaker iframes are controlled by reveal.js and never drive live reload.
  if (window.parent !== window) return;
  const generation = Number(document.querySelector('meta[name="adocweave-preview-generation"]').content);
  let pending = false;
  async function update() {
    if (pending) return;
    pending = true;
    try {
      const event = await fetch("/events", { cache: "no-store" }).then(response => response.json());
      if (event.generation !== generation) {
        window.location.reload();
        return;
      }
      const diagnostics = event.diagnostics;
      const display = document.querySelector(".slides-diagnostics");
      if (display) {
        const errors = diagnostics.filter(item => item.severity === "error" || item.code === "preview-build");
        display.textContent = errors.map(item => `${item.code}: ${item.message}`).join("\n");
        display.hidden = errors.length === 0;
      }
    } catch (_) {
      // The previous complete slide stays visible while the server reconnects.
    } finally {
      pending = false;
    }
  }
  setInterval(update, 500);
  update();
})();
