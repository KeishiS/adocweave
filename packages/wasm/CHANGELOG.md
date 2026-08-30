# Changelog

## [0.54.0] - 2026-08-30

### Breaking changes

- The public `WASM_PACKAGE_VERSION` export has been removed. Read the installed package version from package metadata when needed; it is not a runtime compatibility signal.
- WebAssembly package releases now use their own `wasm/vX.Y.Z` tags and are published directly to npm instead of being attached to the native GitHub Release.

### Maintenance

- Package sources now live under `packages/wasm` together with the package manifest, README, and changelog.
