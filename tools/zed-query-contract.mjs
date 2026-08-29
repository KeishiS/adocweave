import { readFileSync } from "node:fs";
import process from "node:process";

const ROOT = new URL("../", import.meta.url);
const read = (path) => readFileSync(new URL(path, ROOT), "utf8");
const snapshot = JSON.parse(read("tools/zed-query-nodes.json"));
const manifest = read("editors/zed/extension.toml");
const inlineConfig = read("editors/zed/languages/asciidoc_inline/config.toml");
const vendoredRoot = "editors/zed/vendor/tree-sitter-asciidoc";
const vendoredSource = JSON.parse(read(`${vendoredRoot}/source.json`));

function fail(message) {
  throw new Error(message);
}

function grammarSection(name) {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = manifest.match(new RegExp(`\\[grammars\\.${escaped}\\]([\\s\\S]*?)(?=\\n\\[|$)`));
  if (!match) fail(`missing grammar declaration: ${name}`);
  return match[1];
}

function queryNodes(source) {
  return [...source.matchAll(/\(([a-z][a-z0-9_]*)\b/g)].map((match) => match[1]);
}

if (vendoredSource.repository !== "https://github.com/cathaysia/tree-sitter-asciidoc" ||
    vendoredSource.commit !== snapshot.commit || vendoredSource.license !== "Apache-2.0") {
  fail("vendored inline grammar provenance does not match the pinned upstream grammar");
}
const vendoredInline = `${vendoredRoot}/tree-sitter-asciidoc_inline`;
const scanner = read(`${vendoredInline}/src/scanner.c`);
const parser = read(`${vendoredInline}/src/parser.c`);
const monospaceCorpus = read(`${vendoredInline}/test/corpus/monospace.txt`);
if (!scanner.includes("TOKEN_UNCONSTRAINED_MONOSPACE") ||
    !parser.includes("sym__unconstrained_monospace") ||
    !monospaceCorpus.includes("``\\`` and ``a\\`b`` and ``code``")) {
  fail("vendored inline grammar is missing the backslash monospace regression contract");
}

if (!/^name = "AsciiDoc Inline"$/m.test(inlineConfig) || !/^hidden = true$/m.test(inlineConfig)) {
  fail("AsciiDoc inline language must be registered by its injection name and hidden from users");
}

const mainInjections = read("editors/zed/languages/asciidoc/injections.scm");
if (!mainInjections.includes('(#set! injection.language "AsciiDoc Inline")') ||
    mainInjections.includes('(#set! injection.language "asciidoc_inline")')) {
  fail("AsciiDoc inline injections must use the registered language name, not the grammar id");
}

const highlightCaptures = new Set([
  "attribute", "comment", "constant", "emphasis", "emphasis.strong", "keyword", "label",
  "link_text", "link_uri", "number", "property", "punctuation.bracket",
  "punctuation.delimiter", "punctuation.list_marker", "punctuation.special", "string",
  "string.escape", "string.special", "text.literal", "title", "type", "variable.parameter",
]);

for (const [grammar, nodes] of Object.entries(snapshot.grammars)) {
  const section = grammarSection(grammar);
  if (!section.includes(`commit = "${snapshot.commit}"`)) {
    fail(`${grammar} does not use the node snapshot commit`);
  }
  if (!section.includes(`path = "tree-sitter-${grammar}"`)) {
    fail(`${grammar} uses an unexpected repository path`);
  }

  const known = new Set(nodes);
  for (const query of ["highlights.scm", "injections.scm"]) {
    const path = `editors/zed/languages/${grammar}/${query}`;
    const source = read(path);
    for (const node of queryNodes(source)) {
      if (!known.has(node)) fail(`${path} references unknown ${grammar} node: ${node}`);
    }
    const captures = [...source.matchAll(/@([A-Za-z_][A-Za-z0-9_.-]*)/g)].map((match) => match[1]);
    if (query === "injections.scm" && captures.includes("content") && captures.includes("injection.content")) {
      fail(`${path} mixes @content and @injection.content`);
    }
    for (const capture of captures) {
      if (capture.startsWith("_")) continue;
      if (query === "injections.scm") {
        if (!["content", "injection.content", "injection.language"].includes(capture)) {
          fail(`${path} uses unsupported injection capture: @${capture}`);
        }
      } else if (!highlightCaptures.has(capture)) {
        fail(`${path} uses unsupported highlight capture: @${capture}`);
      }
    }
  }
}

process.stdout.write(`Zed query contract verified: ${snapshot.commit}\n`);
