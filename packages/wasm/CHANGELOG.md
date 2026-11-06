# Changelog

## [0.56.0] - 2026-11-05

### Breaking changes

- Titled paragraphs, lists, math, passthrough, and unstyled quote containers group their titles and bodies in `div.titled-block`. Authored IDs and allowed role classes move to that wrapper; update CSS selectors accordingly. Titled literal paragraphs use `figure.literal-block` with a `figcaption`.
- Unstyled quote containers without titles carry explicit IDs and allowed roles on a `div` without a fixed kind class.
- Source, listing, and literal captions render inline elements instead of flattening their titles to plain text. Document-structure title strings remain plain text.
- Comment block titles no longer participate in semantic analysis: they do not execute counters or contribute references, footnotes, citations, or formulas to analysis results.

### Fixed

- HTML output retains titles on supported content blocks, including paragraphs, lists, quotations, literal paragraphs, math, and passthrough.
- Title links, references, footnotes, citations, and formulas use the ordinary inline renderer, preserving footnote references and backlinks alongside the title.
- Verse paragraph titles are displayed before their paragraphs. Paragraph-only verse bodies preserve their line breaks; verse blocks containing other block kinds use ordinary block rendering.

## [0.55.0] - 2026-10-04

### Breaking changes

- `MathLanguage` adds `"asciimath"`, and generic STEM formulas select their notation from the `stem` attribute at their source position. Unset or empty `stem` now selects AsciiMath; use `:stem: latexmath` or explicit `latexmath` notation for LaTeX.
- The public protocol schema version increases from 16 to 17. Use JavaScript and WebAssembly artifacts from the same package release so the compatibility check succeeds.
- Math blocks' `formulas[].sourceRange` now includes preceding block metadata. Use `contentRange` when selecting only the formula text.

### Fixed

- Math block styles combined with IDs, roles, or options and explicit `asciimath` inline and block notation are recognized consistently.
- HTML formula output preserves escaped source text and identifies LaTeX or AsciiMath through its class and `data-math-language` attribute.

## [0.54.1] - 2026-08-30

### Breaking changes

- The Worker and direct entry points now use one `analyze({ source, products, resources? })` request and result contract. Results contain only the requested products.
- `projection` is renamed to `document`, whose result type is `DocumentView`. The former `products`, `parse`, and `renderDiagnostics` result fields are removed.
- The former `process` and separate `preprocess` entry points are removed. `AnalyzeRequest` requires a `source` object and a non-empty `products` object; unknown fields, invalid enum values, `null`, and `false` product values are rejected.
- Worker cancellation and all other Worker or direct-entry failures now use `AdocWeaveError`. Cancellation no longer uses `DOMException`, and errors are no longer thrown as JSON strings.
- The public `WASM_PACKAGE_VERSION` export has been removed. Read the installed package version from package metadata when needed; it is not a runtime compatibility signal.
- WebAssembly package releases now use their own `wasm/vX.Y.Z` tags and are published directly to npm instead of being attached to the native GitHub Release.

### Migration

Replace the former top-level fields with the structured request: move `sourceId` to `source.id`, syntax settings to `source`, diagnostic settings to `products.diagnostics`, HTML settings to `products.html`, and resolved inputs to `resources`.

Before:

```javascript
const result = await analyze({
  sourceId: "guide.adoc",
  source,
  products: { html: true, projection: true },
  renderPolicy: { documentMode: "fragment" },
});
```

After:

```javascript
const result = await analyze({
  source: { id: "guide.adoc", text: source },
  products: {
    html: { documentMode: "fragment" },
    document: true,
  },
});
```

Check whether a requested product field exists before reading it, and catch `AdocWeaveError` for cancellation and other failures.

### Maintenance

- Package sources now live under `packages/wasm` together with the package manifest, README, and changelog.
