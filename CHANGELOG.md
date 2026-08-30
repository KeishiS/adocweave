# Changelog

## [0.53.0] - 2026-08-30

### Main changes

- Language Server analysis now consumes one owned `ProjectRequest` per document revision. Open primary documents and include files use the captured editor contents, while closed includes return to filesystem contents.
- Primary-source language features remain available when include expansion fails. Non-file document URIs are rejected as `unsupported-uri` without filesystem access.
- Project source identifiers are opaque values mapped to LSP URIs only at the protocol boundary. Diagnostics, navigation, hover, completion, formatting, symbols, links, rename, and semantic tokens share the same adopted project result.

### Rust API

- `ConfigSelection::Resolved` accepts an `Arc<ResolvedProjectConfig>`. Live callers can freeze and share configuration with the rest of a request instead of discovering or rereading a project file during processing.
- `ProjectResourceResult.requested_at` identifies the source and optional source range that requested a related resource.

### Migration

- Construct `ProjectResourceResult` with `requested_at`. Use `None` for resources without a requesting source, and use `ProjectSourceLocation.range = None` when no authored position exists.

## [0.52.0] - 2026-08-30

### Rust API

- `ProjectTargetResult.analysis` now separates primary-source analysis from include expansion. A successful `ProjectAnalysis` contains the unexpanded `Analysis` in `primary`; its `expanded` result contains `ProjectExpandedAnalysis` or a typed `ProjectExpansionError`. A missing or rejected include no longer discards primary analysis.
- Non-cancellation parse failures use the new `ProjectParseError`. Primary parse failures are `ProjectTargetError::Parse`; expansion failures are the flat `ProjectExpansionError::{Options, Preprocess, Parse, Projection, Resource, Incomplete}` variants. Cancellation is always returned as request-wide `ProjectError::Cancelled`.

### Migration

- Replace the former `target.outcome` result with `target.analysis`. After unwrapping that result, replace `.source` with `.primary`; the former `.preprocessed`, `.source_mapping`, and `.local_target_diagnostics` fields move under the `.expanded` result. No compatibility aliases are provided.
- Replace matches on the former `ProjectTargetError::Analysis` with `ProjectTargetError::Parse` for primary parsing and the corresponding `ProjectExpansionError` variant for expansion. Handle cancellation from `process` as `ProjectError::Cancelled` instead of a target-local analysis error.

```rust
let analysis = target.analysis?;
use_primary(&analysis.primary);
if let Ok(expanded) = analysis.expanded {
    use_preprocessed(&expanded.preprocessed);
    use_source_mapping(&expanded.source_mapping);
    use_local_target_diagnostics(&expanded.local_target_diagnostics);
}
```

## [0.51.0] - 2026-08-29

### Main changes

- The CLI and Language Server now use a single `adocweave` executable. Start the Language Server with `adocweave lsp`.
- All artifacts now share one version, one `vX.Y.Z` tag, and one GitHub Release.
- The Browser Worker and Node.js direct entry now use one WebAssembly request, result, and `AdocWeaveError` contract.
- WebAssembly results contain only the products selected by the request.
- Multi-file `format --write` and `check --fix` prepare every change before replacing files. Each file is replaced without leaving partial contents; if a later replacement fails, earlier replacements are not rolled back.
- Live preview now uses the same project request as one-shot CLI commands for documents, includes, stylesheets, configuration, limits, and cancellation.

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
- `adocweave-project` now owns its authority, limits, usage, resource observations, configuration view, and error contracts. Requests use the core `SourceId`, support file overlays and pathless input, accept a `CancellationCheck`, and can resolve configuration without analyzing documents. The former `adocweave-config` and `adocweave-host` types are no longer exposed by this API.
- `ProjectRequest` is consumed by each stateless processing call. `ProjectAuthority::observation_access`, resource observations, and `ProjectError::repair_candidate` let live callers detect changes through the same retained filesystem authority. `ProjectTarget::PathNoSymlinks` supports callers that must reject symbolic links in an authored target path.
- Rust crate versions now follow the repository-wide version.

[0.53.0]: https://github.com/KeishiS/adocweave/releases/tag/v0.53.0
[0.52.0]: https://github.com/KeishiS/adocweave/releases/tag/v0.52.0
[0.51.0]: https://github.com/KeishiS/adocweave/releases/tag/v0.51.0
