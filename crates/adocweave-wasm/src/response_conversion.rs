//! Direct conversion from core output types to the public WASM wire contract.

use adocweave::output::diagnostics::{
    Applicability as CoreApplicability, Diagnostic as CoreDiagnostic, Severity as CoreSeverity,
    sort_diagnostics,
};
use adocweave::output::projection::{
    self, BlockPresentationKind as CoreBlockPresentationKind, FormulaKind as CoreFormulaKind,
    ProjectedText, SearchTextKind as CoreSearchTextKind,
};
use adocweave::resolution::{
    ReferenceKey as CoreReferenceKey, ResolutionFailureKind, ResolutionNoticeKind,
    ResolutionOutcome,
};
use adocweave::semantic::{
    DocumentSymbol as CoreDocumentSymbol, MathLanguage as CoreMathLanguage,
    OrderedListStyle as CoreOrderedListStyle, ReferenceTargetKind as CoreReferenceTargetKind,
    SectionKind as CoreSectionKind, SymbolKind as CoreSymbolKind, TocEntry as CoreTocEntry,
};

use crate::response_wire::*;
use crate::{MathLanguage, Severity};

pub(crate) fn wasm_diagnostics(diagnostics: &[CoreDiagnostic]) -> Vec<Diagnostic> {
    let mut diagnostics = diagnostics.to_vec();
    sort_diagnostics(&mut diagnostics);
    diagnostics
        .into_iter()
        .map(|diagnostic| Diagnostic {
            id: diagnostic.id.as_str().to_owned(),
            code: diagnostic.code.as_str().to_owned(),
            severity: match diagnostic.severity {
                CoreSeverity::Error => Severity::Error,
                CoreSeverity::Warning => Severity::Warning,
                CoreSeverity::Information => Severity::Information,
                CoreSeverity::Hint => Severity::Hint,
            },
            message: diagnostic.message,
            range: wasm_text_range(diagnostic.range),
            related: diagnostic
                .related
                .into_iter()
                .map(|related| RelatedInformation {
                    range: wasm_text_range(related.range),
                    message: related.message,
                })
                .collect(),
            fixes: diagnostic
                .fixes
                .into_iter()
                .map(|fix| {
                    let edits = fix
                        .edits()
                        .iter()
                        .map(|edit| TextEdit {
                            range: wasm_text_range(edit.range),
                            replacement: edit.replacement.clone(),
                        })
                        .collect();
                    Fix {
                        title: fix.title,
                        applicability: match fix.applicability {
                            CoreApplicability::Always => Applicability::Always,
                            CoreApplicability::Maybe => Applicability::Maybe,
                        },
                        edits,
                    }
                })
                .collect(),
        })
        .collect()
}

pub(crate) fn wasm_document_symbols(symbols: Vec<CoreDocumentSymbol>) -> Vec<DocumentSymbol> {
    symbols.into_iter().map(wasm_document_symbol).collect()
}

fn wasm_document_symbol(symbol: CoreDocumentSymbol) -> DocumentSymbol {
    DocumentSymbol {
        name: symbol.name,
        kind: match symbol.kind {
            CoreSymbolKind::DocumentTitle => SymbolKind::DocumentTitle,
            CoreSymbolKind::Part => SymbolKind::Part,
            CoreSymbolKind::Section => SymbolKind::Section,
            CoreSymbolKind::ListItem => SymbolKind::ListItem,
        },
        range: wasm_text_range(symbol.range),
        selection_range: wasm_text_range(symbol.selection_range),
        children: symbol
            .children
            .into_iter()
            .map(wasm_document_symbol)
            .collect(),
    }
}

pub(crate) fn wasm_document_projection(
    analysis: &adocweave::Analysis,
    inputs: &adocweave::resolution::RenderInputs,
) -> DocumentView {
    let headings = analysis
        .structure()
        .headings()
        .iter()
        .map(|heading| {
            let presentation = analysis
                .presentation()
                .heading_at(heading.range)
                .expect("every projected heading has presentation facts");
            StructuredHeading {
                kind: section_kind(heading.kind),
                level: u32::from(heading.level),
                id: heading.id.clone(),
                id_range: wasm_text_range(heading.id_range),
                title: heading.title.clone(),
                range: wasm_text_range(heading.range),
                title_range: wasm_text_range(heading.title_range),
                number: presentation.number.clone(),
                toc_included: presentation.toc_included,
            }
        })
        .collect();
    let toc = analysis
        .presentation()
        .toc()
        .iter()
        .map(wasm_toc_entry)
        .collect();
    let manpage = analysis.structure().manpage().map(|manpage| Manpage {
        name: manpage.name.clone(),
        section: manpage.section.clone(),
        purpose: manpage.purpose.clone(),
        title_range: wasm_text_range(manpage.title_range),
        name_range: wasm_text_range(manpage.name_range),
        purpose_range: wasm_text_range(manpage.purpose_range),
    });
    let footnotes = analysis
        .catalogs()
        .footnotes()
        .iter()
        .map(|footnote| Footnote {
            number: footnote.number,
            id: footnote.id.clone(),
            definition_range: wasm_text_range(footnote.definition_range),
            content_range: wasm_text_range(footnote.content_range),
            text: footnote.text.clone(),
            occurrences: footnote
                .occurrences
                .iter()
                .map(|occurrence| wasm_text_range(occurrence.range))
                .collect(),
        })
        .collect();
    let bibliography = analysis
        .catalogs()
        .bibliography()
        .iter()
        .map(|entry| BibliographyEntry {
            id: entry.id.clone(),
            label: entry.label.clone(),
            definition_range: wasm_text_range(entry.definition_range),
            references: entry
                .references
                .iter()
                .map(|reference| wasm_text_range(reference.range))
                .collect(),
        })
        .collect();
    let index = analysis
        .catalogs()
        .index()
        .iter()
        .map(|entry| IndexEntry {
            terms: entry.terms.clone(),
            display: entry.display.clone(),
            occurrences: entry
                .occurrences
                .iter()
                .copied()
                .map(wasm_text_range)
                .collect(),
        })
        .collect();

    DocumentView {
        source_id: analysis
            .source_id()
            .map(|source_id| source_id.as_str().to_owned()),
        source_blocks: projection::source_blocks(analysis)
            .into_iter()
            .map(|source| SourceBlock {
                source_range: wasm_text_range(source.source_range),
                content_range: wasm_text_range(source.content_range),
                title: source.title.map(wasm_projected_text),
                language_range: source.language_range.map(wasm_text_range),
                language: source.language,
                line_numbers: source.line_numbers,
                start_line: source.start_line,
                source: source.source,
                caption: source.caption,
            })
            .collect(),
        formulas: projection::formulas(analysis)
            .into_iter()
            .map(|formula| Formula {
                kind: match formula.kind {
                    CoreFormulaKind::Inline => FormulaKind::Inline,
                    CoreFormulaKind::Block => FormulaKind::Block,
                },
                language: math_language(formula.language),
                source_range: wasm_text_range(formula.source_range),
                content_range: wasm_text_range(formula.content_range),
                source: formula.source,
            })
            .collect(),
        citations: analysis
            .citations()
            .into_iter()
            .map(|citation| Citation {
                order: citation.order,
                source_range: wasm_text_range(citation.range),
                keys: citation
                    .keys
                    .into_iter()
                    .map(|key| CitationKey {
                        source_range: wasm_text_range(key.range),
                        key: key.value,
                    })
                    .collect(),
                attributes: citation
                    .attributes
                    .into_iter()
                    .map(|attribute| CitationAttribute {
                        source_range: wasm_text_range(attribute.range),
                        name: attribute.name,
                        value: attribute.value,
                    })
                    .collect(),
            })
            .collect(),
        block_presentations: projection::block_presentations(analysis)
            .into_iter()
            .map(|block| BlockPresentation {
                kind: block_presentation_kind(block.kind),
                source_range: wasm_text_range(block.source_range),
                content_range: wasm_text_range(block.content_range),
                title: block.title,
                attribution: block.attribution,
                citation: block.citation,
                roles: block.roles,
                open: block.open,
                caption: block.caption,
            })
            .collect(),
        ordered_lists: projection::ordered_lists(analysis)
            .into_iter()
            .map(|list| OrderedList {
                source_range: wasm_text_range(list.source_range),
                start: list.start,
                reversed: list.reversed,
                style: ordered_list_style(list.style),
            })
            .collect(),
        reference_edges: projection::reference_edges(analysis, inputs)
            .into_iter()
            .map(|edge| ReferenceEdge {
                source_id: edge
                    .source_id
                    .map(|source_id| source_id.as_str().to_owned()),
                source_range: wasm_text_range(edge.source_range),
                target: reference_key(edge.target),
                resolution: edge.resolution.map(resolution_outcome),
            })
            .collect(),
        external_links: projection::external_links(analysis)
            .into_iter()
            .map(|link| ExternalLink {
                source_range: wasm_text_range(link.source_range),
                target_range: wasm_text_range(link.target_range),
                target: link.target,
                label: link.label,
            })
            .collect(),
        searchable_text: {
            let searchable = projection::searchable_text(analysis);
            SearchableText {
                text: searchable.text,
                segments: searchable
                    .segments
                    .into_iter()
                    .map(|segment| SearchTextSegment {
                        kind: match segment.kind {
                            CoreSearchTextKind::Prose => SearchTextKind::Prose,
                            CoreSearchTextKind::Code => SearchTextKind::Code,
                        },
                        source_range: wasm_text_range(segment.source_range),
                        text: segment.text,
                    })
                    .collect(),
            }
        },
        structure: DocumentStructure {
            headings,
            toc,
            manpage,
        },
        catalogs: DocumentCatalogs {
            footnotes,
            bibliography,
            index,
        },
        targets: analysis
            .reference_targets()
            .iter()
            .map(|target| ReferenceTarget {
                kind: reference_target_kind(target.kind),
                id: target.id.clone(),
                label: target.label.clone(),
                id_range: wasm_text_range(target.id_range),
                target_range: wasm_text_range(target.target_range),
            })
            .collect(),
        title: projection::document_title(analysis).map(wasm_projected_text),
    }
}

fn wasm_projected_text(text: ProjectedText) -> DocumentText {
    DocumentText {
        source_range: wasm_text_range(text.source_range),
        text: text.text,
    }
}

fn wasm_toc_entry(entry: &CoreTocEntry) -> TocEntry {
    TocEntry {
        id: entry.id.clone(),
        title: entry.title.clone(),
        level: u32::from(entry.level),
        number: entry.number.clone(),
        range: wasm_text_range(entry.range),
        children: entry.children.iter().map(wasm_toc_entry).collect(),
    }
}

const fn math_language(language: CoreMathLanguage) -> MathLanguage {
    match language {
        CoreMathLanguage::Latex => MathLanguage::Latex,
        CoreMathLanguage::Typst => MathLanguage::Typst,
    }
}

const fn section_kind(kind: CoreSectionKind) -> SectionKind {
    match kind {
        CoreSectionKind::DocumentTitle => SectionKind::DocumentTitle,
        CoreSectionKind::Part => SectionKind::Part,
        CoreSectionKind::Section => SectionKind::Section,
        CoreSectionKind::Appendix => SectionKind::Appendix,
        CoreSectionKind::Discrete => SectionKind::Discrete,
    }
}

const fn reference_target_kind(kind: CoreReferenceTargetKind) -> ReferenceTargetKind {
    match kind {
        CoreReferenceTargetKind::DocumentTitle => ReferenceTargetKind::DocumentTitle,
        CoreReferenceTargetKind::Part => ReferenceTargetKind::Part,
        CoreReferenceTargetKind::Section => ReferenceTargetKind::Section,
        CoreReferenceTargetKind::ExplicitAnchor => ReferenceTargetKind::ExplicitAnchor,
        CoreReferenceTargetKind::InlineAnchor => ReferenceTargetKind::InlineAnchor,
    }
}

const fn block_presentation_kind(kind: CoreBlockPresentationKind) -> BlockPresentationKind {
    match kind {
        CoreBlockPresentationKind::Admonition => BlockPresentationKind::Admonition,
        CoreBlockPresentationKind::Quote => BlockPresentationKind::Quote,
        CoreBlockPresentationKind::Verse => BlockPresentationKind::Verse,
        CoreBlockPresentationKind::Example => BlockPresentationKind::Example,
        CoreBlockPresentationKind::Sidebar => BlockPresentationKind::Sidebar,
        CoreBlockPresentationKind::Open => BlockPresentationKind::Open,
        CoreBlockPresentationKind::Collapsible => BlockPresentationKind::Collapsible,
        CoreBlockPresentationKind::Figure => BlockPresentationKind::Figure,
        CoreBlockPresentationKind::Table => BlockPresentationKind::Table,
    }
}

const fn ordered_list_style(style: CoreOrderedListStyle) -> OrderedListStyle {
    match style {
        CoreOrderedListStyle::Arabic => OrderedListStyle::Arabic,
        CoreOrderedListStyle::Decimal => OrderedListStyle::Decimal,
        CoreOrderedListStyle::LowerAlpha => OrderedListStyle::Loweralpha,
        CoreOrderedListStyle::UpperAlpha => OrderedListStyle::Upperalpha,
        CoreOrderedListStyle::LowerRoman => OrderedListStyle::Lowerroman,
        CoreOrderedListStyle::UpperRoman => OrderedListStyle::Upperroman,
        CoreOrderedListStyle::LowerGreek => OrderedListStyle::Lowergreek,
    }
}

fn reference_key(key: CoreReferenceKey) -> ReferenceKey {
    match key {
        CoreReferenceKey::Local { anchor } => ReferenceKey::Local { anchor },
        CoreReferenceKey::Document { document, anchor } => {
            ReferenceKey::Document { document, anchor }
        }
        CoreReferenceKey::Scheme {
            scheme,
            locator,
            anchor,
        } => ReferenceKey::Scheme {
            scheme,
            locator,
            anchor,
        },
    }
}

fn resolution_outcome(outcome: ResolutionOutcome) -> DocumentResolutionOutcome {
    match outcome {
        ResolutionOutcome::Resolved {
            href,
            display_text,
            notices,
        } => DocumentResolutionOutcome::Resolved {
            href,
            display_text,
            notices: notices
                .into_iter()
                .map(|notice| match notice.kind {
                    ResolutionNoticeKind::Fallback => {
                        DocumentReferenceNotice::ReferenceResolutionFallback
                    }
                })
                .collect(),
        },
        ResolutionOutcome::Failed(failure) => DocumentResolutionOutcome::Failed {
            kind: match failure.kind {
                ResolutionFailureKind::MissingTarget => {
                    DocumentReferenceFailureKind::MissingReferenceTarget
                }
                ResolutionFailureKind::MissingAnchor => {
                    DocumentReferenceFailureKind::MissingReferenceAnchor
                }
                ResolutionFailureKind::AmbiguousTarget => {
                    DocumentReferenceFailureKind::AmbiguousReferenceTarget
                }
                ResolutionFailureKind::OutsideRoot => {
                    DocumentReferenceFailureKind::ReferenceOutsideRoot
                }
                ResolutionFailureKind::ResolverFailure => {
                    DocumentReferenceFailureKind::ReferenceResolverFailure
                }
            },
        },
    }
}

pub(crate) fn wasm_text_range(range: adocweave::text::TextRange) -> TextRange {
    TextRange {
        start: range.start().to_u32(),
        end: range.end().to_u32(),
    }
}

#[cfg(test)]
mod tests {
    use adocweave::resolution::{ResolutionNotice, ResolverFailure};

    use super::*;

    #[test]
    fn every_core_projection_enum_has_the_intended_wire_variant() {
        for (core, wire) in [
            (CoreMathLanguage::Latex, MathLanguage::Latex),
            (CoreMathLanguage::Typst, MathLanguage::Typst),
        ] {
            assert_eq!(math_language(core), wire);
        }
        for (core, wire) in [
            (CoreSectionKind::DocumentTitle, SectionKind::DocumentTitle),
            (CoreSectionKind::Part, SectionKind::Part),
            (CoreSectionKind::Section, SectionKind::Section),
            (CoreSectionKind::Appendix, SectionKind::Appendix),
            (CoreSectionKind::Discrete, SectionKind::Discrete),
        ] {
            assert_eq!(section_kind(core), wire);
        }
        for (core, wire) in [
            (
                CoreReferenceTargetKind::DocumentTitle,
                ReferenceTargetKind::DocumentTitle,
            ),
            (CoreReferenceTargetKind::Part, ReferenceTargetKind::Part),
            (
                CoreReferenceTargetKind::Section,
                ReferenceTargetKind::Section,
            ),
            (
                CoreReferenceTargetKind::ExplicitAnchor,
                ReferenceTargetKind::ExplicitAnchor,
            ),
            (
                CoreReferenceTargetKind::InlineAnchor,
                ReferenceTargetKind::InlineAnchor,
            ),
        ] {
            assert_eq!(reference_target_kind(core), wire);
        }
        for (core, wire) in [
            (
                CoreBlockPresentationKind::Admonition,
                BlockPresentationKind::Admonition,
            ),
            (
                CoreBlockPresentationKind::Quote,
                BlockPresentationKind::Quote,
            ),
            (
                CoreBlockPresentationKind::Verse,
                BlockPresentationKind::Verse,
            ),
            (
                CoreBlockPresentationKind::Example,
                BlockPresentationKind::Example,
            ),
            (
                CoreBlockPresentationKind::Sidebar,
                BlockPresentationKind::Sidebar,
            ),
            (CoreBlockPresentationKind::Open, BlockPresentationKind::Open),
            (
                CoreBlockPresentationKind::Collapsible,
                BlockPresentationKind::Collapsible,
            ),
            (
                CoreBlockPresentationKind::Figure,
                BlockPresentationKind::Figure,
            ),
            (
                CoreBlockPresentationKind::Table,
                BlockPresentationKind::Table,
            ),
        ] {
            assert_eq!(block_presentation_kind(core), wire);
        }
        for (core, wire) in [
            (CoreOrderedListStyle::Arabic, OrderedListStyle::Arabic),
            (CoreOrderedListStyle::Decimal, OrderedListStyle::Decimal),
            (
                CoreOrderedListStyle::LowerAlpha,
                OrderedListStyle::Loweralpha,
            ),
            (
                CoreOrderedListStyle::UpperAlpha,
                OrderedListStyle::Upperalpha,
            ),
            (
                CoreOrderedListStyle::LowerRoman,
                OrderedListStyle::Lowerroman,
            ),
            (
                CoreOrderedListStyle::UpperRoman,
                OrderedListStyle::Upperroman,
            ),
            (
                CoreOrderedListStyle::LowerGreek,
                OrderedListStyle::Lowergreek,
            ),
        ] {
            assert_eq!(ordered_list_style(core), wire);
        }
    }

    #[test]
    fn every_reference_resolution_variant_has_the_intended_wire_variant() {
        for (core, wire) in [
            (
                ResolutionFailureKind::MissingTarget,
                DocumentReferenceFailureKind::MissingReferenceTarget,
            ),
            (
                ResolutionFailureKind::MissingAnchor,
                DocumentReferenceFailureKind::MissingReferenceAnchor,
            ),
            (
                ResolutionFailureKind::AmbiguousTarget,
                DocumentReferenceFailureKind::AmbiguousReferenceTarget,
            ),
            (
                ResolutionFailureKind::OutsideRoot,
                DocumentReferenceFailureKind::ReferenceOutsideRoot,
            ),
            (
                ResolutionFailureKind::ResolverFailure,
                DocumentReferenceFailureKind::ReferenceResolverFailure,
            ),
        ] {
            assert_eq!(
                resolution_outcome(ResolutionOutcome::Failed(ResolverFailure { kind: core })),
                DocumentResolutionOutcome::Failed { kind: wire },
            );
        }

        assert_eq!(
            resolution_outcome(ResolutionOutcome::Resolved {
                href: "#target".to_owned(),
                display_text: Some("Target".to_owned()),
                notices: vec![ResolutionNotice {
                    kind: ResolutionNoticeKind::Fallback,
                }],
            }),
            DocumentResolutionOutcome::Resolved {
                href: "#target".to_owned(),
                display_text: Some("Target".to_owned()),
                notices: vec![DocumentReferenceNotice::ReferenceResolutionFallback],
            },
        );
    }
}
