// MathJax-specific output conventions stay at this adapter boundary.
// Public helper results contain only SVG and diagnostics, never these internal nodes.
export function localFragment(value) {
  if (typeof value !== "string" || !value.startsWith("#")) return null;
  try { return decodeURIComponent(value.slice(1)); } catch { return null; }
}

export function prepareMathTree(data, footnote) {
  let numbered = footnote && Object.keys(data.tags.labels).length > 0;
  data.root.walkTree((node) => {
    node.attributes?.unset("data-latex");
    node.attributes?.unset("data-latex-item");
    if (footnote && node.kind === "mlabeledtr") numbered = true;
  });
  return numbered;
}

export function equationReferences(root) {
  const references = [];
  root.walkTree((node) => {
    if (node.attributes?.get("class") === "MathJax_ref") references.push(localFragment(node.attributes.get("href")));
  });
  return references;
}

export function finishSvg(adaptor, root, visit) {
  // MathJax normally supplies these rules in its page stylesheet. Keep SVG
  // self-contained, including nested viewports and array frame lines.
  visit(root, (node) => {
    // Background rectangles retain their paint; this MathJax marker is not needed for display.
    adaptor.removeAttribute(node, "data-bgcolor");
    // fcolorbox already draws its frame as polygons; the mpadded CSS border is redundant.
    if (adaptor.kind(node) === "g" && adaptor.getAttribute(node, "data-mml-node") === "mpadded") {
      const style = adaptor.getAttribute(node, "style");
      if (style) {
        const retained = style.split(";").filter(part => part.trim() && !/^\s*border\s*:/.test(part)).join(";");
        if (retained) adaptor.setAttribute(node, "style", retained);
        else adaptor.removeAttribute(node, "style");
      }
    }
    if (adaptor.kind(node) === "svg") adaptor.setStyle(node, "overflow", "visible");
    if (adaptor.getAttribute(node, "data-line") || adaptor.getAttribute(node, "data-frame")) {
      adaptor.setAttribute(node, "stroke-width", "70");
      adaptor.setAttribute(node, "fill", "none");
      const classes = adaptor.getAttribute(node, "class") ?? "";
      if (classes.includes("mjx-dashed")) adaptor.setAttribute(node, "stroke-dasharray", "140");
      if (classes.includes("mjx-dotted")) {
        adaptor.setAttribute(node, "stroke-dasharray", "0,140");
        adaptor.setAttribute(node, "stroke-linecap", "round");
      }
    }
  });
  adaptor.setStyle(root, "min-height", "1px");
  adaptor.setAttribute(root, "aria-hidden", "true");
}
