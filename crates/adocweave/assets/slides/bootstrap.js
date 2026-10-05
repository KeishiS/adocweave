/* Fixed AdocWeave initialization; authored metadata is never evaluated. */
(() => {
  "use strict";
  const presenter = document.body.dataset.audience === "presenter";
  const deck = new Reveal(document.querySelector(".reveal"), {
    width: 1280,
    height: 720,
    hash: true,
    fragmentInURL: true,
    history: false,
    center: false,
    transition: "none",
    slideNumber: "c/t",
    plugins: presenter ? [RevealNotes] : []
  });
  window.Reveal = deck;

  function navigate(target) {
    const slide = target.closest(".slides section");
    if (!slide) return false;
    const indices = deck.getIndices(slide);
    const fragment = target.closest(".fragment");
    const value = fragment ? Number(fragment.dataset.fragmentIndex) : -1;
    const current = deck.getIndices();
    const required = Number.isInteger(value) ? value : -1;
    const vertical = indices.v ?? 0;
    const sameSlide = current.h === indices.h && current.v === vertical;
    const stage = sameSlide && Number.isInteger(current.f) ? Math.max(current.f, required) : required;
    deck.slide(indices.h, vertical, stage);
    // Reveal may expose all fragments when returning to a previous slide.
    // Apply the requested stage after its slide transition has finished.
    deck.navigateFragment(stage);
    return true;
  }

  document.addEventListener("click", event => {
    const anchor = event.target.closest && event.target.closest("a[href^='#']");
    if (!anchor) return;
    let id;
    try { id = decodeURIComponent(anchor.getAttribute("href").slice(1)); } catch { return; }
    const target = document.getElementById(id);
    if (target && navigate(target)) {
      event.preventDefault();
      // Handle authored references before Reveal's bubbling hash listener.
      event.stopPropagation();
    }
  }, true);

  function imageReady(image) {
    if (image.decode) return image.decode().catch(() => {});
    if (image.complete) return Promise.resolve();
    return new Promise(resolve => {
      image.addEventListener("load", resolve, { once: true });
      image.addEventListener("error", resolve, { once: true });
    });
  }

  deck.initialize().then(async () => {
    const fonts = document.fonts ? document.fonts.ready : Promise.resolve();
    await Promise.all([fonts, ...Array.from(document.images, imageReady)]);
    deck.layout();
    if (!location.hash.startsWith("#/")) {
      let id;
      try { id = decodeURIComponent(location.hash.slice(1)); } catch { id = ""; }
      const target = document.getElementById(id);
      if (target) navigate(target);
    }
    window.dispatchEvent(new Event("adocweave-slides-ready"));
  });
})();
