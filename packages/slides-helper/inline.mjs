import { parseFragment } from "parse5";

export class InlineError extends Error {}

const tags = new Map([
  ["i", "emphasis"], ["em", "emphasis"], ["b", "strong"], ["strong", "strong"],
  ["sup", "superscript"], ["sub", "subscript"],
]);
const styles = new Map([
  ["font-variant:small-caps", "smallcaps"],
  ["font-style:normal", "normal-emphasis"],
  ["font-weight:normal", "normal-strong"],
  ["font-variant:normal", "normal-smallcaps"],
  ["text-decoration:underline", "underline"],
]);
const wrappers = new Set(["csl-entry", "csl-left-margin", "csl-right-inline", "csl-block", "csl-indent"]);

function safeLink(href) {
  if (typeof href !== "string" || Buffer.byteLength(href) > 4096 || !/^https?:\/\//i.test(href) || /[\u0000-\u0020\u007f]/.test(href)) return false;
  try {
    const url = new URL(href);
    return ["http:", "https:"].includes(url.protocol) && !url.username && !url.password;
  } catch {
    return false;
  }
}

// Size the host's finite HTML representation without serializing HTML here.
// The shared Rust/Node contract cases cover escaping and wrapper overhead.
const wrapperBytes = new Map([
  ["emphasis", "<em></em>"], ["strong", "<strong></strong>"],
  ["superscript", "<sup></sup>"], ["subscript", "<sub></sub>"],
  ["smallcaps", '<span class="csl-smallcaps"></span>'],
  ["normal-emphasis", '<span class="csl-normal-emphasis"></span>'],
  ["normal-strong", '<span class="csl-normal-strong"></span>'],
  ["normal-smallcaps", '<span class="csl-normal-smallcaps"></span>'],
  ["underline", "<u></u>"], ["link", '<a href=""></a>'],
].map(([kind, markup]) => [kind, Buffer.byteLength(markup)]));

function escapedBytes(value) {
  let bytes = Buffer.byteLength(value);
  for (const character of value) {
    switch (character) {
      case "&": bytes += 4; break;
      case "<": case ">": bytes += 3; break;
      case '"': bytes += 5; break;
      case "'": bytes += 4; break;
    }
  }
  return bytes;
}

function validateOutput(nodes) {
  let count = 0;
  let textBytes = 0;
  let htmlBytes = 0;
  const check = (nodes, depth = 0) => {
    if (depth > 32) throw new InlineError("CSL output exceeds the inline tree limit.");
    for (const node of nodes) {
      if (++count > 4096) throw new InlineError("CSL output exceeds the inline tree limit.");
      if (node.kind === "text") {
        textBytes += Buffer.byteLength(node.text);
        htmlBytes += escapedBytes(node.text);
      } else {
        htmlBytes += wrapperBytes.get(node.kind);
        if (node.kind === "link") htmlBytes += escapedBytes(node.href);
      }
      if (textBytes > 256 * 1024 || htmlBytes > 6 * 256 * 1024) {
        throw new InlineError("CSL output exceeds the inline byte limit.");
      }
      if (node.children) check(node.children, depth + 1);
    }
  };
  check(nodes);
}

function normalize(nodes) {
  const result = [];
  for (const node of nodes) {
    if (node.kind === "text" && node.text === "") continue;
    const previous = result.at(-1);
    if (node.kind === "text" && previous?.kind === "text") previous.text += node.text;
    else result.push(node);
  }
  return result;
}

export function parseInlines(html, bibliography = false) {
  let count = 0;
  const convert = (node, depth) => {
    if (++count > 32768 || depth > 32) throw new InlineError("CSL output exceeds the inline tree limit.");
    if (node.nodeName === "#text") return [{ kind: "text", text: node.value }];
    if (node.nodeName === "#document-fragment") return children(node, depth);
    const attrs = Object.fromEntries((node.attrs ?? []).map((attr) => [attr.name, attr.value]));
    const names = Object.keys(attrs);
    if (tags.has(node.tagName) && names.length === 0) {
      return [{ kind: tags.get(node.tagName), children: children(node, depth) }];
    }
    if (node.tagName === "span" && names.length === 1) {
      if (attrs.class === "nocase") return children(node, depth);
      const style = attrs.style?.replace(/\s/g, "").replace(/;$/, "");
      if (styles.has(style)) return [{ kind: styles.get(style), children: children(node, depth) }];
    }
    if (node.tagName === "a" && names.length === 1 && safeLink(attrs.href)) {
      return [{ kind: "link", href: attrs.href, children: children(node, depth) }];
    }
    if (bibliography && node.tagName === "div" && names.length === 1 && wrappers.has(attrs.class)) {
      const content = children(node, depth);
      // Numeric styles put the label and text in separate divs. Retain a
      // readable separator after flattening those known citeproc wrappers.
      if (["csl-left-margin", "csl-block", "csl-indent"].includes(attrs.class)) content.push({ kind: "text", text: " " });
      return content;
    }
    throw new InlineError(`Unsupported CSL HTML element or attributes: ${node.tagName ?? node.nodeName}.`);
  };
  const children = (node, depth) => normalize((node.childNodes ?? []).flatMap((child) => convert(child, depth + 1)));
  const result = normalize(convert(parseFragment(html), 0));
  validateOutput(result);
  return result;
}
