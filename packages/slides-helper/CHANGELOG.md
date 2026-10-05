# Changelog

## [0.2.0] - 2026-10-05

### Added

- Initial release: a one-request JSON executable using schema version 3 for static TeX SVG and CSL citations and bibliographies in independent body and presenter-note scopes. Successful responses contain every result; failed responses contain diagnostics only.
- Complete inline expressions render in a single static SVG, including operators, right-hand sides, and equation references. Footnote equations permit unnumbered inline math and references to equations outside footnotes, while rejecting display equations, labels, and numbering.
- Standalone SVG preserves clipping for stretched braces and delimiters, with visible overflow limited to the root and MathJax's equation-layout viewports.
- Requests explicitly select the bundled `color`, `cancel`, and `mathtools` extensions. AMS numbering and references remain available with an empty selection. Color output retains paint values for host validation and removes MathJax's internal background marker.
- Bounded input and output, finite citation formatting, and distribution notices for the libraries and fonts actually used. CSL inputs must be nonempty and normalized citation output stays within the host's inline node budget.
- An npm archive containing the complete audited runtime dependency tree as bundled dependencies.

[0.2.0]: https://github.com/KeishiS/adocweave/tree/slides-helper/v0.2.0
