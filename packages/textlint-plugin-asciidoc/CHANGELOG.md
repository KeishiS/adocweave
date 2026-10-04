# Changelog

## [0.54.1] - 2026-10-04

### Fixed

- Explicit AsciiMath formulas and math blocks whose style shares a metadata line with an ID, role, or option are excluded from prose linting. Inline formulas use the existing `Code` nodes, preserving the Processor API and TxtAST types.

## [0.54.0] - 2026-08-30

### Breaking changes

- Releases now use independent `textlint-plugin-asciidoc/vX.Y.Z` tags and publish directly to npm instead of using the native GitHub Release.

### Maintenance

- `package.json` and this changelog now define the package version and release history independently of the Cargo workspace.
