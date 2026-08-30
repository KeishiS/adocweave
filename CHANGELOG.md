# Changelog

## [0.56.0] - 2026-08-30

### Main changes

- Native GitHub Releases contain only four cargo-dist archives, their individual SHA-256 files, `sha256.sum`, the standard manifest, and attestations. Cachix is the only external publication started by the native Release workflow.
- Native version updates, native installation acceptance, platform checks, and workflow policies are owned by their distribution instead of a repository-wide release registry and shared installation path.
- The Language Server analyzes open documents and their transitive include and local-reference targets instead of scanning every AsciiDoc file below workspace folders.
- Workspace folders define configuration-search and filesystem-authority roots. The innermost containing folder wins; a file folder uses its parent directory as authority.
- The `workspace.scan` project setting is removed. Old configurations are rejected instead of accepted through an alias or warning.
- Language Server analysis now consumes one owned `ProjectRequest` per document revision. Open primary documents and include files use the captured editor contents, while closed includes return to filesystem contents.
- Primary-source language features remain available when include expansion fails. Non-file document URIs are rejected as `unsupported-uri` without filesystem access.
- Project source identifiers are opaque values mapped to LSP URIs only at the protocol boundary. Diagnostics, navigation, hover, completion, formatting, symbols, links, rename, and semantic tokens share the same adopted project result.
- Language Server project results and errors pass through the same document-generation and filesystem-observation checks. Analysis state, diagnostics, and watched references are adopted together, so stale workers cannot partially update the active document.
- Include, local-target, and project-configuration observations are owned per open document. File notifications reanalyze only affected documents, while oversized notifications reanalyze each open document once without scanning the workspace.
- The former workspace scan, recovery coordinator, scan generations, pending-change journal, and duplicate workspace resource state are removed.
- Project configuration types, TOML validation, defaults, relative-path resolution, and schema generation now belong to `adocweave-project`. The separate `adocweave-config` crate is removed.
- The unused `adocweave-workspace` crate is removed. One-shot project processing belongs to `adocweave-project`, while Language Server session state remains in `adocweave-lsp`.

### Rust API

- `ProjectTarget::Workspace` is removed. Use `ProjectTarget::Directory` or `ProjectTarget::Glob` when a caller explicitly requests multi-file discovery.
- `ConfigSelection` and `ProjectOverrides` are replaced by `ProjectConfigSelection` and `ProjectConfigOverrides`.
- `ProjectConfig` is the effective configuration type. `ResolvedProjectConfig`, `ConfigSelection::Resolved`, `ProjectScopeId`, and `AnalysisSnapshotBudget` are removed without compatibility aliases.
- `ProjectLimits` stores file count and size ceilings in `ProjectResourceLimits`, the same type returned by `ProjectConfig::resource_limits`.
- `ProjectResourceResult.requested_at` identifies the source and optional source range that requested a related resource.
- `HostReferenceIndex`, `HostReferenceRequest`, `NoHostReferenceIndex`, and `run_with_host_index` are removed from `adocweave-lsp`.
- `Workspace`, `WorkspaceSnapshot`, workspace analysis drafts, resource revisions, generations, dependency graphs, and retained-resource budgets from `adocweave-workspace` are removed without compatibility aliases.

### Migration

- Remove `[workspace.scan]` from `.adocweave.toml`. There is no replacement because the Language Server no longer performs a workspace scan.
- Replace `ProjectTarget::Workspace` with an explicit file, directory, glob, or in-memory source target.
- Construct `ProjectResourceResult` with `requested_at`. Use `None` for resources without a requesting source, and use `ProjectSourceLocation.range = None` when no authored position exists.
- Use `adocweave_lsp::run` or `adocweave_lsp::run_stdio` to start the Language Server. Product-specific reference indexes must be implemented outside the Language Server protocol adapter.
- Replace `ConfigSelection` with `ProjectConfigSelection` and `ProjectOverrides` with `ProjectConfigOverrides`. Remove callers which construct or inject `ResolvedProjectConfig`; project processing now obtains configuration through `ProjectConfigSelection` and `ProjectAuthority`.
- Move the `max_files`, `max_read_bytes`, and `max_resource_bytes` fields of `ProjectLimits` into `ProjectLimits::resources`, renaming `max_read_bytes` to `max_total_bytes`.
- Replace `adocweave-workspace` usage with an owned `adocweave_project::ProjectRequest`. Language Server integrations should keep open-document revisions and adopted dependencies in their session instead of introducing a shared workspace state manager.

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

[0.56.0]: https://github.com/KeishiS/adocweave/releases/tag/v0.56.0
[0.52.0]: https://github.com/KeishiS/adocweave/releases/tag/v0.52.0
[0.51.0]: https://github.com/KeishiS/adocweave/releases/tag/v0.51.0
