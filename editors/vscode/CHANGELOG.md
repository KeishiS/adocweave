# Changelog

## [0.55.1] - 2026-08-30

### Changed

- The extension display name is now `AdocWeave Language Support` to avoid a Marketplace conflict with the former extension. The extension ID remains `adocweave.adocweave`.

## [0.55.0] - 2026-08-30

### Changed

- The extension ID is now `adocweave.adocweave`. Remove the former extension and install the new ID because registry clients cannot migrate between extension IDs automatically.
- The extension now has its own version and `vscode/vX.Y.Z` release tag. Its version no longer matches the native AdocWeave release by design.
- AdocWeave 0.51.0 or later is required. Available editor features are determined from the Language Server capabilities announced during initialization.
