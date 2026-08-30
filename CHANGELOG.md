# Changelog

## [0.56.0] - 2026-08-30

### Main changes

- The CLI and Language Server now use a single `adocweave` executable. Start the Language Server with `adocweave lsp`; the former `adocweave-lsp` executable is removed.
- CLI commands, options, English help, argument errors, and shell completions now come from one `clap` definition. On Unix, file arguments no longer need to be valid UTF-8.
- `adocweave rules` replaces `check --list-rules`. Use `--format json` instead of `check --json`, `format --diff` or `check --fix --diff` instead of `--dry-run`, and `--stdin-base` instead of `--base-dir` for standard input.
- `check --project-root` now enables local-reference validation within that root. The redundant `--local-targets` option is removed.
- Multi-file `format --write` and `check --fix` prepare every change before replacing files. A replacement never leaves a partial file; if a later replacement fails, earlier completed replacements are not rolled back.
- Live preview now uses the same project request as one-shot CLI commands for documents, includes, stylesheets, configuration, limits, and cancellation.
- Native GitHub Releases contain only four cargo-dist archives, their individual SHA-256 files, `sha256.sum`, the standard manifest, and attestations. Cachix is the only external publication started by the native Release workflow.
- The Language Server analyzes open documents and their transitive include and local-reference targets instead of scanning every AsciiDoc file below workspace folders.
- Workspace folders define configuration-search and filesystem-authority roots. The innermost containing folder wins; a file workspace folder uses its parent directory as authority.
- The `workspace.scan` setting and the former scan, recovery, generation, journal, and duplicate workspace-resource state are removed. File notifications reanalyze only affected open documents.
- Language Server analysis now consumes one owned `ProjectRequest` per document revision. Open primary documents and include files use the captured editor contents, while closed includes return to filesystem contents.
- Primary-source language features remain available when include expansion fails. Non-file document URIs are rejected as `unsupported-uri` without filesystem access.
- Project source identifiers are opaque values mapped to LSP URIs only at the protocol boundary. Diagnostics, navigation, hover, completion, formatting, symbols, links, rename, and semantic tokens share the same adopted project result.
- Language Server project results and errors pass through the same document-generation and filesystem-observation checks. Analysis state, diagnostics, and watched references are adopted together, so stale workers cannot partially update the active document.
- Include, local-target, and project-configuration observations are owned per open document. File notifications reanalyze only affected documents, while oversized notifications reanalyze each open document once without scanning the workspace.
- `adocweave-project` now owns project configuration, bounded local-file access, explicit target discovery, safe replacement, and one-shot processing. Paths are normalized before configuration search and authority checks. The separate `adocweave-config`, `adocweave-host`, and `adocweave-workspace` crates are removed.
- CLI process exit codes now belong to the CLI. The Language Server reports protocol or runtime failures without depending directly on `adocweave-host`.
- The environment-independent Rust library is renamed from `adocweave` to `adocweave-core`. The CLI package is renamed from `adocweave-cli` to `adocweave`; the installed executable remains `adocweave`.
- A successful project result retains primary-source analysis when include expansion fails. Expanded contents and their source mapping are available only from the separate expansion result.

### Rust API

- `ProjectTarget::Workspace` is removed. Use `ProjectTarget::Directory` or `ProjectTarget::Glob` when a caller explicitly requests multi-file discovery.
- `ConfigSelection` and `ProjectOverrides` are replaced by `ProjectConfigSelection` and `ProjectConfigOverrides`.
- `ProjectConfig` is the effective configuration type. `ResolvedProjectConfig`, `ConfigSelection::Resolved`, `ProjectScopeId`, and `AnalysisSnapshotBudget` are removed without compatibility aliases.
- `ProjectLimits` stores file count and size ceilings in `ProjectResourceLimits`, the same type returned by `ProjectConfig::resource_limits`.
- `ProjectResourceResult.requested_at` identifies the source and optional source range that requested a related resource.
- `HostReferenceIndex`, `HostReferenceRequest`, `NoHostReferenceIndex`, and `run_with_host_index` are removed from `adocweave-lsp`.
- `Workspace`, `WorkspaceSnapshot`, workspace analysis drafts, resource revisions, generations, dependency graphs, and retained-resource budgets from `adocweave-workspace` are removed without compatibility aliases.
- `adocweave_host::ExitStatus` is removed. The CLI uses its private `CliExitCode`; `adocweave_lsp::StdioError::kind` returns `StdioErrorKind::Protocol` or `StdioErrorKind::Runtime`.
- `ProjectObservationSession` is replaced by `ProjectObserver`, and `ProjectObservationAccess::session` is replaced by `observer`. Project source identifiers use `adocweave_core::SourceId`; the former `LogicalSourceId` and all `adocweave-host` APIs are removed without compatibility aliases.
- The `adocweave` library crate and its `adocweave::` Rust path are removed. Use the `adocweave-core` package and the `adocweave_core::` Rust path. No dependency alias or compatibility re-export is provided.
- The former `adocweave-cli` package is removed. The `adocweave` package now owns the CLI and the `adocweave` executable.
- `ProjectTargetResult.analysis` contains a `ProjectAnalysis` with unexpanded analysis in `primary` and a separate `expanded` result. Primary parse failures use `ProjectTargetError::Parse`; expansion failures use `ProjectExpansionError`, while request cancellation remains `ProjectError::Cancelled`.
- `adocweave-project` owns authority, limits, usage, resource observations, configuration, and error contracts. Requests use `adocweave_core::SourceId`, support editor overlays and pathless input, accept a `CancellationCheck`, and can resolve configuration without analyzing a document.
- `ProjectRequest` is consumed by each stateless processing call. `ProjectAuthority::observation_access` and `ProjectError::repair_candidate` identify related filesystem changes so a caller can decide when to retry.
- `ProjectTarget::PathNoSymlinks` rejects symbolic links in an explicitly selected path.
- `adocweave-wasm` accepts the public Serde `AnalyzeRequest` directly and returns `AnalyzeResult`. The old `Wasm*` request types, normalization layer, and separate preprocessing request are removed.
- The core output API no longer exports WASM-only serializers, test-only conformance helpers, CLI-only diagnostic renderers, HTML allowlists, internal text-role classifiers, or a duplicate `ResolvedReference`. Use `adocweave_core::resolution::ResolvedReference`.

### Migration

- Replace direct `adocweave-lsp` invocations with `adocweave lsp`. Replace Nix uses of `apps.${system}.adocweave-lsp` or `nix run ...#adocweave-lsp` with the default app followed by `-- lsp`.
- Download the native executable from the `v0.56.0` Release instead of the former `adocweave-cli/vX.Y.Z` and `adocweave-lsp/vX.Y.Z` Releases. Each platform archive now contains only `adocweave`.
- Pin Rust workspace APIs with `v0.56.0` instead of the former `adocweave-lib/vX.Y.Z` tag.
- Replace `check --list-rules` with `rules`, `check --json` with `check --format json`, `format --write --dry-run` with `format --diff`, and `check --fix --dry-run` with `check --fix --diff`. Replace `--base-dir` with `--stdin-base` when reading standard input; for file input, remove `--base-dir` because includes resolve from each document's parent directory. Remove `--local-targets`; specifying `--project-root` enables local-reference validation.
- Remove `[workspace.scan]` from `.adocweave.toml`. There is no replacement because the Language Server no longer performs a workspace scan.
- Replace `ProjectTarget::Workspace` with an explicit file, directory, glob, or in-memory source target.
- Construct `ProjectResourceResult` with `requested_at`. Use `None` for resources without a requesting source, and use `ProjectSourceLocation.range = None` when no authored position exists.
- Use `adocweave_lsp::run` or `adocweave_lsp::run_stdio` to start the Language Server. Product-specific reference indexes must be implemented outside the Language Server protocol adapter.
- Replace `ConfigSelection` with `ProjectConfigSelection` and `ProjectOverrides` with `ProjectConfigOverrides`. Remove callers which construct or inject `ResolvedProjectConfig`; project processing now obtains configuration through `ProjectConfigSelection` and `ProjectAuthority`.
- Move the `max_files`, `max_read_bytes`, and `max_resource_bytes` fields of `ProjectLimits` into `ProjectLimits::resources`, renaming `max_read_bytes` to `max_total_bytes`.
- Replace `adocweave-workspace` usage with an owned `adocweave_project::ProjectRequest`. Language Server integrations should keep open-document revisions and adopted dependencies in their session instead of introducing a shared workspace state manager.
- Replace `StdioError::exit_status` with `StdioError::kind` when embedding the Language Server. Process exit-code policy belongs to the embedding executable.
- Replace `ProjectObservationSession` with `ProjectObserver` and call `ProjectObservationAccess::observer`. Replace `LogicalSourceId` with `adocweave_core::SourceId`. Remove direct `adocweave-host` dependencies; local filesystem access is performed as part of `ProjectRequest` processing.
- Replace Rust dependencies on `adocweave` with `adocweave-core` and imports from `adocweave::` with `adocweave_core::`. APIs formerly exported only for WASM, CLI, tests, or core internals have no compatibility re-export; use `adocweave_core::resolution::ResolvedReference` for resolved references. Replace workspace commands such as `cargo build -p adocweave-cli` with `cargo build -p adocweave`; the executable and `adocweave lsp` command remain unchanged.
- Replace `target.outcome` with `target.analysis` and `.source` with `.primary`. Read `.preprocessed`, `.source_mapping`, and `.local_target_diagnostics` from the `.expanded` result. Match primary parse failures as `ProjectTargetError::Parse`, expansion failures as `ProjectExpansionError`, and cancellation as `ProjectError::Cancelled`.

```rust
let analysis = target.analysis?;
use_primary(&analysis.primary);
if let Ok(expanded) = analysis.expanded {
    use_preprocessed(&expanded.preprocessed);
    use_source_mapping(&expanded.source_mapping);
    use_local_target_diagnostics(&expanded.local_target_diagnostics);
}
```

[0.56.0]: https://github.com/KeishiS/adocweave/releases/tag/v0.56.0
