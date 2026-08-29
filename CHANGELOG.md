# Changelog

## [0.51.0] - 2026-08-29

### Main changes

- The CLI and Language Server now use a single `adocweave` executable. Start the Language Server with `adocweave lsp`.
- All artifacts now share one version, one `vX.Y.Z` tag, and one GitHub Release.

### Migration

- Replace direct `adocweave-lsp` invocations with `adocweave lsp`. The old executable and a compatibility alias are not provided.
- Download all artifacts from the common `vX.Y.Z` Release instead of product-specific Releases.

### Rust API

There are no incompatible changes to the public Rust API in this release. Rust crate versions now follow the repository-wide version.

[0.51.0]: https://github.com/KeishiS/adocweave/releases/tag/v0.51.0
