//! What one rendering pass needs besides the blocks themselves.

use crate::catalog::DocumentCatalogs;
use crate::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticId, Severity};
use crate::document::DocumentIdentifiers;
use crate::presentation::DocumentPresentation;
use crate::render::{RenderInputUsage, RenderInputs};
use crate::source::TextRange;

use super::TerminalPolicy;

/// The document-wide facts a block or an inline needs while it is laid out,
/// together with the problems found on the way.
pub(super) struct RenderContext<'document, 'inputs> {
    pub(super) policy: &'document TerminalPolicy,
    pub(super) presentation: &'document DocumentPresentation,
    pub(super) identifiers: &'document DocumentIdentifiers,
    pub(super) catalogs: &'document DocumentCatalogs,
    pub(super) usage: RenderInputUsage<'inputs>,
    pub(super) diagnostics: Vec<Diagnostic>,
}

impl<'document, 'inputs> RenderContext<'document, 'inputs> {
    pub(super) fn new(
        policy: &'document TerminalPolicy,
        document: &'document crate::block_model::AstDocument,
        inputs: &'inputs RenderInputs,
    ) -> Self {
        Self {
            policy,
            presentation: document.presentation(),
            identifiers: document.identifiers(),
            catalogs: document.catalogs(),
            usage: inputs.track_usage(),
            diagnostics: Vec::new(),
        }
    }

    /// The problems found while laying the document out, together with the
    /// host resolutions that were never used or were given twice.
    pub(super) fn finish(self) -> Vec<Diagnostic> {
        let mut diagnostics = self.diagnostics;
        for problem in self.usage.finish() {
            let domain = problem.domain.as_str();
            let (code, message) = match problem.kind {
                crate::render::RenderInputProblemKind::Duplicate => (
                    "duplicate-render-input",
                    format!("multiple {domain} resolutions have the same source range"),
                ),
                crate::render::RenderInputProblemKind::Unused => (
                    "unused-render-input",
                    format!("{domain} resolution does not match a renderable {domain}"),
                ),
            };
            diagnostics.push(Diagnostic {
                id: DiagnosticId::new(format!(
                    "{code}@{}:{}",
                    problem.range.start().to_u32(),
                    problem.range.end().to_u32()
                )),
                code: DiagnosticCode::new(code),
                severity: Severity::Warning,
                message,
                range: problem.range,
                related: Vec::new(),
                fixes: Vec::new(),
            });
        }
        diagnostics
    }

    /// Records a problem found while laying the document out. It carries the
    /// same code as the HTML backend reports for the same problem, so a host
    /// that shows both sees one vocabulary.
    pub(super) fn report(&mut self, code: &str, message: &str, range: TextRange) {
        self.diagnostics.push(Diagnostic {
            id: DiagnosticId::new(format!(
                "{code}@{}:{}",
                range.start().to_u32(),
                range.end().to_u32()
            )),
            code: DiagnosticCode::new(code),
            severity: Severity::Warning,
            message: message.to_owned(),
            range,
            related: Vec::new(),
            fixes: Vec::new(),
        });
    }
}
