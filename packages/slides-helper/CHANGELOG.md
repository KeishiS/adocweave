# Changelog

## [0.2.0] - 2026-10-05

### Changed

- Use schema version 2: a successful response contains every result; a failed response contains diagnostics only.
- Limit normalized citation output to the host's inline node budget and require nonempty CSL inputs at both boundaries.
- Mark footnote equations explicitly and reject display equations, labels, and numbering while preserving references to body equations.

## [0.1.1] - 2026-10-05

### Fixed

- Preserve complete inline expressions in a single static SVG, including operators, right-hand sides, and equation references.

## [0.1.0] - 2026-10-04

### Added

- One-request JSON executable for static TeX SVG and CSL citations and bibliographies in independent body and speaker-note scopes.
- Bounded input and output, finite citation formatting, and distribution notices for the libraries and fonts actually used.
- npm archive containing the complete audited runtime dependency tree as bundled dependencies.

[0.2.0]: https://github.com/KeishiS/adocweave/compare/slides-helper/v0.1.1...slides-helper/v0.2.0
[0.1.1]: https://github.com/KeishiS/adocweave/compare/slides-helper/v0.1.0...slides-helper/v0.1.1
[0.1.0]: https://github.com/KeishiS/adocweave/tree/slides-helper/v0.1.0
