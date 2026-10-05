# Changelog

## [0.2.0] - 2026-10-05

### Added

- Initial release: a one-request JSON executable using schema version 2 for static TeX SVG and CSL citations and bibliographies in independent body and presenter-note scopes. Successful responses contain every result; failed responses contain diagnostics only.
- Complete inline expressions render in a single static SVG, including operators, right-hand sides, and equation references. Footnote equations permit unnumbered inline math and references to equations outside footnotes, while rejecting display equations, labels, and numbering.
- Bounded input and output, finite citation formatting, and distribution notices for the libraries and fonts actually used. CSL inputs must be nonempty and normalized citation output stays within the host's inline node budget.
- An npm archive containing the complete audited runtime dependency tree as bundled dependencies.

[0.2.0]: https://github.com/KeishiS/adocweave/tree/slides-helper/v0.2.0
