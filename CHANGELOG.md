# Changelog

## [0.51.0] - 2026-08-29

### Main changes

- The CLI and Language Server now use a single `adocweave` executable. Start the Language Server with `adocweave lsp`.
- All artifacts now share one version, one `vX.Y.Z` tag, and one GitHub Release.
- The Browser Worker and Node.js direct entry now use one WebAssembly request, result, and `AdocWeaveError` contract.
- WebAssembly results contain only the products selected by the request.

### Migration

- Replace direct `adocweave-lsp` invocations with `adocweave lsp`. The old executable and a compatibility alias are not provided.
- Download all artifacts from the common `vX.Y.Z` Release instead of product-specific Releases.
- Replace the former top-level WebAssembly fields with `source`, `products`, and optional `resources`. Move `sourceId` to `source.id`, syntax settings to `source`, diagnostic settings to `products.diagnostics`, HTML settings to `products.html`, and resolved inputs to `resources`.
- Replace the `projection` product and result with `document`. The result type is now `DocumentView`.
- Check whether a requested product field exists instead of reading empty values for unrequested products. The `products`, `parse`, and `renderDiagnostics` result fields have been removed.
- Catch `AdocWeaveError` for Browser cancellation and all other Browser or Node.js failures. Errors are no longer thrown as JSON strings, and cancellation no longer uses `DOMException`.

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

### JavaScript API

- `AnalyzeRequest` requires a `source` object and a non-empty `products` object. Unknown fields, `null`, `false` product values, invalid enum values, and invalid ranges are rejected.
- Browser Worker and Node.js direct analysis use the same generated `AnalyzeRequest` and `AnalyzeResult` declarations.
- The old `process` and separate `preprocess` WebAssembly exports are replaced by `analyze`.

### Rust API

- `adocweave-wasm` now accepts the public Serde `AnalyzeRequest` directly and returns `AnalyzeResult`. The old `Wasm*` request types, normalization layer, and separate preprocessing request have been removed.
- The core `adocweave::output` API no longer exports the WASM-only canonical AST and syntax serializers, test-only conformance snapshot helpers, CLI-only diagnostic renderers, HTML allowlists, internal text-role classifiers, or a duplicate HTML-path `ResolvedReference`. Use `adocweave::resolution::ResolvedReference` for resolved references; canonical products remain available through `adocweave-wasm`, while diagnostic display and lint-catalog JSON belong to the CLI.
- Rust crate versions now follow the repository-wide version.

[0.51.0]: https://github.com/KeishiS/adocweave/releases/tag/v0.51.0
