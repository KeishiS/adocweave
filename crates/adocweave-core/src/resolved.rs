//! Construction and ownership of immutable document-wide semantic facts.
//!
//! The raw semantic tree is complete before this model is built. Dependencies
//! between derived views are passed explicitly so no consumer can observe a
//! partially resolved document.

use crate::attributes::AttributeEnvironment;
use crate::block_model::AstDocument;
use crate::limits::AnalysisLimits;

/// Immutable, source-ordered facts collected from a semantic document in one pass.
///
/// Facts are independent of output backends and host resolution. Derived views
/// such as catalogs, reference queries, and resource queries consume this
/// index instead of traversing the document tree again.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DocumentFacts {
    attribute_references: Vec<crate::attributes::AttributeReference>,
    links: Vec<crate::inline_model::Link>,
    references: Vec<crate::inline_model::Reference>,
    macros: Vec<crate::inline_model::StandardMacro>,
    resources: Vec<crate::resource::ResourceReference>,
    footnote_bodies: Vec<FootnoteBody>,
}

/// The inline content of one footnote definition.
///
/// A footnote body is prose written inside a macro, so the inline tree keeps
/// it as the macro's attribute text and this fact carries the parsed content.
/// Bodies are keyed by the macro range so catalogs and renderers can find the
/// content of a definition without a second parse.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FootnoteBody {
    macro_range: crate::source::TextRange,
    inlines: Vec<crate::inline_model::Inline>,
    problems: Vec<crate::inline_model::InlineProblem>,
}

impl DocumentFacts {
    fn build(
        document: &AstDocument,
        attributes: &AttributeEnvironment,
        limits: AnalysisLimits,
        checkpoint: &mut crate::cancellation::CancellationCheckpoint<'_>,
    ) -> Result<Self, ()> {
        let mut facts = Self::default();
        for binding in attributes.bindings() {
            if checkpoint.is_cancelled() {
                return Err(());
            }
            for reference in crate::attributes::value_references(binding, attributes) {
                if checkpoint.is_cancelled() {
                    return Err(());
                }
                facts.attribute_references.push(reference);
            }
        }
        let walked = crate::walker::try_walk_ast(document, |node| {
            if checkpoint.is_cancelled() {
                return std::ops::ControlFlow::Break(());
            }
            match node {
                crate::walker::SemanticNode::Inline(
                    crate::inline_model::Inline::AttributeReference {
                        range,
                        name_range,
                        name,
                        ..
                    },
                ) => facts.attribute_references.push(attribute_reference(
                    name,
                    *range,
                    *name_range,
                    attributes,
                )),
                crate::walker::SemanticNode::Inline(crate::inline_model::Inline::Link(link)) => {
                    facts.links.push(link.clone());
                    for reference in &link.target_attributes {
                        if checkpoint.is_cancelled() {
                            return std::ops::ControlFlow::Break(());
                        }
                        facts
                            .attribute_references
                            .push(attribute_use(reference, attributes));
                    }
                }
                crate::walker::SemanticNode::Inline(crate::inline_model::Inline::Reference(
                    reference,
                )) => {
                    facts.references.push(reference.clone());
                    for reference in &reference.target_attributes {
                        if checkpoint.is_cancelled() {
                            return std::ops::ControlFlow::Break(());
                        }
                        facts
                            .attribute_references
                            .push(attribute_use(reference, attributes));
                    }
                }
                crate::walker::SemanticNode::Inline(crate::inline_model::Inline::Macro(node)) => {
                    facts.macros.push(node.clone());
                    match footnote_body(node, attributes, limits, checkpoint) {
                        Ok(Some(body)) => facts.footnote_bodies.push(body),
                        Ok(None) => {}
                        Err(()) => return std::ops::ControlFlow::Break(()),
                    }
                    for reference in &node.target_attributes {
                        if checkpoint.is_cancelled() {
                            return std::ops::ControlFlow::Break(());
                        }
                        facts
                            .attribute_references
                            .push(attribute_use(reference, attributes));
                    }
                    for resource in crate::resource::ResourceReference::from_macro(node) {
                        if checkpoint.is_cancelled() {
                            return std::ops::ControlFlow::Break(());
                        }
                        facts.resources.push(resource);
                    }
                }
                _ => {}
            }
            std::ops::ControlFlow::Continue(())
        });
        if walked.is_break() {
            return Err(());
        }
        crate::cancellation::sort_by_cancellable(
            &mut facts.attribute_references,
            &mut |left, right| left.range.start().cmp(&right.range.start()),
            checkpoint,
        )?;
        crate::cancellation::sort_by_cancellable(
            &mut facts.links,
            &mut |left, right| left.range.start().cmp(&right.range.start()),
            checkpoint,
        )?;
        crate::cancellation::sort_by_cancellable(
            &mut facts.references,
            &mut |left, right| left.range.start().cmp(&right.range.start()),
            checkpoint,
        )?;
        crate::cancellation::sort_by_cancellable(
            &mut facts.macros,
            &mut |left, right| left.range.start().cmp(&right.range.start()),
            checkpoint,
        )?;
        crate::cancellation::sort_by_cancellable(
            &mut facts.resources,
            &mut |left, right| left.range().start().cmp(&right.range().start()),
            checkpoint,
        )?;
        crate::cancellation::sort_by_cancellable(
            &mut facts.footnote_bodies,
            &mut |left, right| left.macro_range.start().cmp(&right.macro_range.start()),
            checkpoint,
        )?;
        Ok(facts)
    }

    /// Parsed inline content of the footnote definition written as `macro_range`.
    pub(crate) fn footnote_body(
        &self,
        macro_range: crate::source::TextRange,
    ) -> Option<&[crate::inline_model::Inline]> {
        self.footnote_bodies
            .binary_search_by(|body| body.macro_range.start().cmp(&macro_range.start()))
            .ok()
            .map(|index| self.footnote_bodies[index].inlines.as_slice())
    }

    /// Inline problems found inside footnote bodies, in source order.
    pub(crate) fn footnote_body_problems(
        &self,
    ) -> impl Iterator<Item = crate::inline_model::InlineProblem> + '_ {
        self.footnote_bodies
            .iter()
            .flat_map(|body| body.problems.iter().copied())
    }
    pub fn attribute_references(&self) -> &[crate::attributes::AttributeReference] {
        &self.attribute_references
    }

    pub fn links(&self) -> &[crate::inline_model::Link] {
        &self.links
    }

    pub fn references(&self) -> &[crate::inline_model::Reference] {
        &self.references
    }

    pub fn macros(&self) -> &[crate::inline_model::StandardMacro] {
        &self.macros
    }

    pub fn resources(&self) -> &[crate::resource::ResourceReference] {
        &self.resources
    }
}

/// Parses a footnote definition's body as inline content.
///
/// The body was kept as raw source text by the inline parser because a macro
/// attribute has no children of its own. Parsing it here, with the document's
/// attribute environment, gives links, formatting, and attribute references in
/// a footnote the same meaning they have in a paragraph. A `\]` the author
/// wrote to keep a bracket inside the body becomes a plain `]`.
fn footnote_body(
    node: &crate::inline_model::StandardMacro,
    attributes: &AttributeEnvironment,
    limits: AnalysisLimits,
    checkpoint: &mut crate::cancellation::CancellationCheckpoint<'_>,
) -> Result<Option<FootnoteBody>, ()> {
    if node.kind != crate::inline_model::StandardMacroKind::Footnote {
        return Ok(None);
    }
    let Some(body) = node.attributes.first() else {
        return Ok(None);
    };
    let Ok(mut budget) = crate::budget::ParseBudget::new(limits) else {
        return Ok(None);
    };
    let config = crate::inline::InlineParseConfig {
        max_depth: limits.max_inline_depth as usize,
        max_formula_bytes: limits.max_formula_bytes as usize,
    };
    let Ok(parsed) =
        crate::inline::parse_with_budget_impl(&body.value, body.value_range, config, &mut budget)
    else {
        return Ok(None);
    };
    let mut inlines = parsed.inlines;
    crate::lowering::resolve_inlines(&mut inlines, attributes, checkpoint)?;
    unescape_closing_brackets(&mut inlines);
    Ok(Some(FootnoteBody {
        macro_range: node.range,
        inlines,
        problems: parsed.problems,
    }))
}

/// Replaces the `\]` an author wrote inside a footnote body with `]`.
///
/// Only text nodes change; every range still addresses the source, so the
/// escaped bracket keeps its position for diagnostics and editors.
fn unescape_closing_brackets(inlines: &mut [crate::inline_model::Inline]) {
    use crate::inline_model::Inline;
    for inline in inlines {
        match inline {
            Inline::Text(text) => {
                if text.value.contains("\\]") {
                    text.value = text.value.replace("\\]", "]");
                }
            }
            Inline::Styled { children, .. } => unescape_closing_brackets(children),
            Inline::Link(link) => unescape_closing_brackets(&mut link.label),
            Inline::Reference(reference) => unescape_closing_brackets(&mut reference.label),
            Inline::Literal { .. }
            | Inline::AttributeReference { .. }
            | Inline::Formula(_)
            | Inline::Macro(_)
            | Inline::Passthrough { .. }
            | Inline::HardBreak { .. } => {}
        }
    }
}

/// Removes the `\]` escape from the plain text of a footnote body.
pub(crate) fn unescape_footnote_text(text: &str) -> String {
    text.replace("\\]", "]")
}

fn attribute_reference(
    name: &str,
    range: crate::source::TextRange,
    name_range: crate::source::TextRange,
    attributes: &AttributeEnvironment,
) -> crate::attributes::AttributeReference {
    // A counter reference resolves the attribute it counts; the reference keeps
    // its authored name so a host can tell the two forms apart.
    let lookup = crate::attributes::counter_reference(name).map_or(name, |counter| counter.name);
    let mut reference = crate::attributes::reference_at(
        lookup,
        range,
        name_range,
        crate::attributes::AttributePosition::new(
            name_range.start(),
            crate::attributes::AttributeEventId::new(u32::MAX),
        ),
        attributes,
    );
    reference.name = name.to_owned();
    reference
}

fn attribute_use(
    reference: &crate::inline_model::AttributeUse,
    attributes: &AttributeEnvironment,
) -> crate::attributes::AttributeReference {
    let start = reference
        .name_range
        .start()
        .to_u32()
        .checked_sub(1)
        .and_then(|value| crate::source::TextSize::new(value as usize).ok())
        .unwrap_or(reference.name_range.start());
    let end = reference
        .name_range
        .end()
        .to_u32()
        .checked_add(1)
        .and_then(|value| crate::source::TextSize::new(value as usize).ok())
        .unwrap_or(reference.name_range.end());
    let range = crate::source::TextRange::new(start, end).unwrap_or(reference.name_range);
    attribute_reference(&reference.name, range, reference.name_range, attributes)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResolvedDocument {
    attribute_environment: AttributeEnvironment,
    facts: DocumentFacts,
    catalogs: crate::catalog::DocumentCatalogs,
    identifiers: crate::document::DocumentIdentifiers,
    structure: crate::structure::DocumentStructure,
    index: crate::presentation::DocumentIndex,
    presentation: crate::presentation::DocumentPresentation,
    layout: crate::presentation::DocumentLayout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResolvedBuildFailure {
    Limit(crate::catalog::CatalogLimitExceeded),
    Cancelled,
}

impl ResolvedDocument {
    pub(crate) fn build(
        document: &AstDocument,
        attributes: AttributeEnvironment,
        catalog_limits: AnalysisLimits,
        checkpoint: &mut crate::cancellation::CancellationCheckpoint<'_>,
    ) -> Result<Self, ResolvedBuildFailure> {
        let facts = DocumentFacts::build(document, &attributes, catalog_limits, checkpoint)
            .map_err(|()| ResolvedBuildFailure::Cancelled)?;
        // Captions come before identifiers: a cross reference to a figure shows
        // the figure's numbered label, so the labels need the numbering.
        let index = crate::presentation::build_index(document, checkpoint)
            .map_err(|()| ResolvedBuildFailure::Cancelled)?;
        let captions = crate::caption::build_captions(document, &index, &attributes, checkpoint)
            .map_err(|()| ResolvedBuildFailure::Cancelled)?;
        let identifiers = crate::document::build_identifiers(document, &captions, checkpoint)
            .map_err(|()| ResolvedBuildFailure::Cancelled)?;
        let structure = crate::structure::build(document, &identifiers, checkpoint)
            .map_err(|()| ResolvedBuildFailure::Cancelled)?;
        let presentation = crate::presentation::build_presentation(
            document,
            &structure,
            &index,
            &attributes,
            captions,
            checkpoint,
        )
        .map_err(|()| ResolvedBuildFailure::Cancelled)?;
        let layout = crate::presentation::build_layout(document, &index, &presentation, checkpoint)
            .map_err(|()| ResolvedBuildFailure::Cancelled)?;
        let catalogs =
            crate::catalog::build(&facts, &index, catalog_limits, checkpoint).map_err(|error| {
                match error {
                    crate::catalog::CatalogBuildFailure::Limit(error) => {
                        ResolvedBuildFailure::Limit(error)
                    }
                    crate::catalog::CatalogBuildFailure::Cancelled => {
                        ResolvedBuildFailure::Cancelled
                    }
                }
            })?;
        Ok(Self {
            attribute_environment: attributes,
            facts,
            catalogs,
            identifiers,
            structure,
            index,
            presentation,
            layout,
        })
    }

    pub(crate) const fn attribute_environment(&self) -> &AttributeEnvironment {
        &self.attribute_environment
    }

    pub(crate) const fn catalogs(&self) -> &crate::catalog::DocumentCatalogs {
        &self.catalogs
    }

    pub(crate) const fn facts(&self) -> &DocumentFacts {
        &self.facts
    }

    pub(crate) const fn identifiers(&self) -> &crate::document::DocumentIdentifiers {
        &self.identifiers
    }

    pub(crate) const fn structure(&self) -> &crate::structure::DocumentStructure {
        &self.structure
    }

    pub(crate) const fn index(&self) -> &crate::presentation::DocumentIndex {
        &self.index
    }

    pub(crate) const fn presentation(&self) -> &crate::presentation::DocumentPresentation {
        &self.presentation
    }

    pub(crate) const fn layout(&self) -> &crate::presentation::DocumentLayout {
        &self.layout
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::DocumentFacts;

    /// A footnote body is parsed with the document's attribute environment, and
    /// an inline problem inside it is reported like one in a paragraph.
    #[test]
    fn footnote_bodies_are_parsed_and_their_problems_become_syntax_issues() {
        let parsed = crate::parser::parse(":who: 筆者\n\nfootnote:[{who} *oops]").expect("parse");
        let node = &parsed.ast.facts().macros()[0];
        let body = parsed
            .ast
            .facts()
            .footnote_body(node.range)
            .expect("footnote body");
        assert!(matches!(
            body.first(),
            Some(crate::inline_model::Inline::AttributeReference { value: Some(value), .. })
                if value == "筆者"
        ));
        assert!(
            parsed
                .syntax
                .issues()
                .iter()
                .any(|issue| issue.class == crate::syntax::SyntaxIssueClass::UnclosedInline),
            "{:?}",
            parsed.syntax.issues()
        );
    }

    #[test]
    fn document_facts_build_cancels_during_the_semantic_walk() {
        struct CancelAfterFirstCheckpoint(AtomicUsize);

        impl crate::core::CancellationCheck for CancelAfterFirstCheckpoint {
            fn is_cancelled(&self) -> bool {
                self.0.fetch_add(1, Ordering::Relaxed) >= 1
            }
        }

        let source = (0..crate::cancellation::CHECKPOINT_INTERVAL * 2)
            .map(|index| format!("https://example.com/{index}[link]\n\n"))
            .collect::<String>();
        let parsed = crate::parser::parse(&source).expect("parse");
        let cancellation = CancelAfterFirstCheckpoint(AtomicUsize::new(0));

        let result = DocumentFacts::build(
            &parsed.ast,
            parsed.ast.attribute_environment(),
            crate::limits::AnalysisLimits::default(),
            &mut crate::cancellation::CancellationCheckpoint::new(&cancellation),
        );

        assert_eq!(result, Err(()));
        assert_eq!(cancellation.0.load(Ordering::Relaxed), 2);
    }
}
