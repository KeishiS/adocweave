import { mathjax } from "@mathjax/src/js/mathjax.js";
import { TeX } from "@mathjax/src/js/input/tex.js";
import { SVG } from "@mathjax/src/js/output/svg.js";
import { liteAdaptor } from "@mathjax/src/js/adaptors/liteAdaptor.js";
import { RegisterHTMLHandler } from "@mathjax/src/js/handlers/html.js";
import "@mathjax/src/js/util/asyncLoad/esm.js";
import "@mathjax/src/js/input/tex/ams/AmsConfiguration.js";
import "@mathjax/src/js/input/tex/newcommand/NewcommandConfiguration.js";
import "@mathjax/src/js/input/tex/configmacros/ConfigMacrosConfiguration.js";

import { diagnostic } from "./protocol.mjs";

const adaptor = liteAdaptor({ fontSize: 16 });
RegisterHTMLHandler(adaptor);

function visitDom(node, action) {
  if (adaptor.kind(node) === "#text") return;
  action(node);
  for (const child of adaptor.childNodes(node)) visitDom(child, action);
}

function localFragment(value) {
  if (typeof value !== "string" || !value.startsWith("#")) return null;
  try {
    return decodeURIComponent(value.slice(1));
  } catch {
    return null;
  }
}

export async function renderEquations(scope, equations, eqnums, macros) {
  const diagnostics = [];
  const failures = new Set();
  const keys = new WeakMap();
  const report = (key, code, message) => {
    failures.add(key);
    if (!diagnostics.some((entry) => entry.key === key && entry.code === code && entry.message === message)) {
      diagnostics.push(diagnostic(scope, key, code, message));
    }
  };
  const tex = new TeX({
    packages: ["base", "ams", "newcommand", "configmacros"],
    tags: eqnums,
    // MathJax's own macro expansion and buffer limits remain enabled.
    macros: Object.fromEntries(macros.map((macro) => [macro.name, macro.default === undefined
      ? [macro.definition, macro.arguments ?? 0]
      : [macro.definition, macro.arguments, macro.default]])),
    formatError(jax, error) {
      report(keys.get(jax.parseOptions.mathItem), "invalid-tex", error.message);
      return jax.formatError(error);
    },
  });
  const svg = new SVG({ fontCache: "local", useXlink: false });
  const document = mathjax.document("", {
    InputJax: tex,
    OutputJax: svg,
    compileError(_document, math, error) {
      report(keys.get(math), "invalid-tex", String(error.message ?? error));
      _document.compileError(math, error);
    },
    typesetError(_document, math, error) {
      report(keys.get(math), "math-typeset-error", String(error.message ?? error));
      _document.typesetError(math, error);
    },
  });
  // Add actual MathItems rather than parsing delimiters in an HTML wrapper.
  // This keeps TeX containing closing delimiters or '<' intact.
  const items = equations.map((equation) => {
    const text = adaptor.text(equation.tex);
    const host = adaptor.node("span", {}, [text]);
    adaptor.append(adaptor.body(document.document), host);
    const math = new document.options.MathItem(equation.tex, tex, equation.display,
      { node: text, n: 0, delim: "" }, { node: text, n: equation.tex.length, delim: "" });
    keys.set(math, equation.key);
    document.math.push(math);
    return math;
  });
  document.processed.set("findMath");
  tex.postFilters.add(({ data }) => {
    data.root.walkTree((node) => {
      node.attributes?.unset("data-latex");
      node.attributes?.unset("data-latex-item");
    });
  });
  await document.renderPromise();
  const ids = new Map();
  const owners = new Map();
  const roots = items.map((math, index) => {
    const root = adaptor.tags(math.typesetRoot, "svg")[0];
    if (!root) return null;
    let id = 0;
    visitDom(root, (node) => {
      const original = adaptor.getAttribute(node, "id");
      if (original) {
        ids.set(original, `${scope}-${equations[index].key}-i${id++}`);
        owners.set(original, equations[index].key);
      }
    });
    return root;
  });
  const references = [];
  for (const [index, math] of items.entries()) {
    const key = equations[index].key;
    math.root.walkTree((node) => {
      if (node.attributes?.get("class") === "MathJax_ref") {
        const target = localFragment(node.attributes.get("href"));
        references.push({ key, target });
        if (!target || !ids.has(target)) {
          report(key, "unresolved-equation-reference", "Equation reference has no numbered target in this scope.");
        }
      }
    });
  }
  // Failed equations are omitted from the response. References to them must
  // also fail, including a chain of references, instead of leaving dead IDs.
  let changed;
  do {
    changed = false;
    for (const { key, target } of references) {
      if (!failures.has(key) && failures.has(owners.get(target))) {
        report(key, "unresolved-equation-reference", "Equation reference points to a failed equation in this scope.");
        changed = true;
      }
    }
  } while (changed);
  const results = items.map((math, index) => {
    const key = equations[index].key;
    const root = roots[index];
    if (!root && !failures.has(key)) report(key, "math-typeset-error", "MathJax produced no SVG result.");
    if (failures.has(key)) return { key, status: "failed" };
    visitDom(root, (node) => {
      const id = adaptor.getAttribute(node, "id");
      if (id) adaptor.setAttribute(node, "id", ids.get(id));
      for (const attribute of ["href", "xlink:href"]) {
        const href = adaptor.getAttribute(node, attribute);
        if (href) {
          const target = localFragment(href);
          if (target && ids.has(target)) adaptor.setAttribute(node, attribute, `#${ids.get(target)}`);
          else report(key, "math-typeset-error", "MathJax produced an unsupported SVG link.");
        }
      }
    });
    if (failures.has(key)) return { key, status: "failed" };
    // MathJax normally supplies these rules in its page stylesheet. Keep the
    // SVG self-contained, including nested viewports and array frame lines.
    visitDom(root, (node) => {
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
    return { key, status: "ok", svg: adaptor.outerHTML(root) };
  });
  return { results, diagnostics };
}
