# Changelog

## [0.55.3] - 2026-11-05

### Fixed

- Pinned the Language Server client to the compatible 9.0.1 release with patched glob dependencies, avoiding resource exhaustion from crafted patterns while preserving the supported editor features.

## [0.55.2] - 2026-10-04

### Fixed

- Updated the runtime `brace-expansion` dependency to fix denial of service from crafted brace patterns.

## [0.55.1] - 2026-08-30

### Changed

- The extension display name is now `AdocWeave Language Support` to avoid a Marketplace conflict with the former extension. The extension ID remains `adocweave.adocweave`.

## [0.55.0] - 2026-08-30

### Changed

- The extension ID is now `adocweave.adocweave`. Remove the former extension and install the new ID because registry clients cannot migrate between extension IDs automatically.
- The extension now has its own version and `vscode/vX.Y.Z` release tag. Its version no longer matches the native AdocWeave release by design.
- AdocWeave 0.51.0 or later is required. Available editor features are determined from the Language Server capabilities announced during initialization.
