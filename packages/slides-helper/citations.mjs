import CSL from "citeproc";

import { parseInlines } from "./inline.mjs";
import { diagnostic } from "./protocol.mjs";

function citationItem(item) {
  const result = { id: item.id };
  for (const field of ["locator", "label", "prefix", "suffix"]) {
    if (Object.hasOwn(item, field)) result[field] = item[field];
  }
  if (item.suppressAuthor) result["suppress-author"] = true;
  if (item.authorOnly) result["author-only"] = true;
  return result;
}

export function renderCitations(scope, citations, csl) {
  const diagnostics = [];
  const results = citations.map(({ key }) => ({ key, status: "failed" }));
  const bibliography = [];
  const items = new Map(csl.items.map((item) => [item.id, item]));
  let activeKey = null;
  const previousDebug = CSL.debug;
  // citeproc's default warning function writes to console.log. Preserve those
  // warnings in the JSON response without polluting the executable's stdout.
  CSL.debug = (message) => diagnostics.push(diagnostic(scope, activeKey, "citeproc-warning", String(message), "warning"));
  try {
    const engine = new CSL.Engine({
      retrieveLocale: () => csl.locale,
      retrieveItem: (id) => {
        if (!items.has(id)) throw new Error(`Missing CSL item: ${id}.`);
        return structuredClone(items.get(id));
      },
    }, csl.style);
    engine.setOutputFormat("html");
    engine.opt.development_extensions.throw_on_empty = true;
    engine.opt.development_extensions.wrap_url_and_doi = true;
    const preceding = [];
    const registered = [];
    for (const [index, citation] of citations.entries()) {
      activeKey = citation.key;
      const missing = citation.items.find((item) => !items.has(item.id));
      if (missing) {
        diagnostics.push(diagnostic(scope, citation.key, "missing-csl-item", `Missing CSL item: ${missing.id}.`));
        continue;
      }
      const input = {
        citationID: citation.key,
        citationItems: citation.items.map(citationItem),
        properties: { noteIndex: engine.opt.xclass === "note" ? index + 1 : 0 },
      };
      const [data, updates] = engine.processCitationCluster(input, preceding, []);
      registered.push(index);
      preceding.push([citation.key, input.properties.noteIndex]);
      for (const [updatedIndex, html] of updates) {
        const resultIndex = registered[updatedIndex];
        if (resultIndex === undefined) throw new Error("citeproc returned an unknown citation index.");
        const key = citations[resultIndex].key;
        try {
          results[resultIndex] = { key, status: "ok", inlines: parseInlines(html) };
        } catch (error) {
          results[resultIndex] = { key, status: "failed" };
          diagnostics.push(diagnostic(scope, key, "unsupported-csl-output", error.message));
        }
      }
      for (const error of data.citation_errors ?? []) {
        diagnostics.push(diagnostic(scope, citation.key, "citation-error", `citeproc could not render item ${error.itemID ?? "<unknown>"}.`));
        results[index] = { key: citation.key, status: "failed" };
      }
    }
    activeKey = null;
    const rendered = engine.makeBibliography();
    if (rendered) {
      const [parameters, entries] = rendered;
      for (const [index, html] of entries.entries()) {
        const ids = parameters.entry_ids[index];
        if (!Array.isArray(ids) || ids.length !== 1 || !items.has(String(ids[0]))) {
          throw new Error("Grouped bibliography entries are not supported.");
        }
        bibliography.push({ id: String(ids[0]), inlines: parseInlines(html, true) });
      }
      for (const error of parameters.bibliography_errors ?? []) {
        diagnostics.push(diagnostic(scope, null, "bibliography-error", `citeproc could not render bibliography item ${error.itemID ?? "<unknown>"}.`));
      }
    }
  } catch (error) {
    // A processor error may leave registry state incomplete. Fail every
    // citation in this scope; the other scope uses an independent engine.
    for (const [index, citation] of citations.entries()) {
      results[index] = { key: citation.key, status: "failed" };
      diagnostics.push(diagnostic(scope, citation.key, "citation-error", String(error.message ?? error)));
    }
    bibliography.length = 0;
  } finally {
    CSL.debug = previousDebug;
  }
  for (const [index, result] of results.entries()) {
    if (diagnostics.some((entry) => entry.key === result.key && entry.severity === "error")) {
      results[index] = { key: result.key, status: "failed" };
    }
    if (result.status === "failed" && !diagnostics.some((entry) => entry.key === result.key && entry.severity === "error")) {
      diagnostics.push(diagnostic(scope, result.key, "citation-error", "citeproc produced no citation result."));
    }
  }
  return { results, bibliography, diagnostics };
}
