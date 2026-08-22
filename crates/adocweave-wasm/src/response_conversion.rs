//! Direct conversion from core output types to the public WASM wire contract.

use adocweave::output::diagnostics::{Applicability, Diagnostic, Severity, sort_diagnostics};
use adocweave::output::projection::{
    BlockPresentationKind, DocumentProjection, FormulaKind, ProjectedText, SearchTextKind,
};
use adocweave::resolution::{
    ReferenceKey, ResolutionFailureKind, ResolutionNoticeKind, ResolutionOutcome,
};
use adocweave::semantic::{
    DocumentSymbol, MathLanguage, OrderedListStyle, ReferenceTargetKind, SectionKind, SymbolKind,
    TocEntry,
};

use crate::response_wire::*;
use crate::{WasmMathLanguage, WasmSeverity};

pub(crate) fn wasm_diagnostics(diagnostics: &[Diagnostic]) -> Vec<WasmDiagnostic> {
    let mut diagnostics = diagnostics.to_vec();
    sort_diagnostics(&mut diagnostics);
    diagnostics
        .into_iter()
        .map(|diagnostic| WasmDiagnostic {
            id: diagnostic.id.as_str().to_owned(),
            code: diagnostic.code.as_str().to_owned(),
            severity: match diagnostic.severity {
                Severity::Error => WasmSeverity::Error,
                Severity::Warning => WasmSeverity::Warning,
                Severity::Information => WasmSeverity::Information,
                Severity::Hint => WasmSeverity::Hint,
            },
            message: diagnostic.message,
            range: wasm_text_range(diagnostic.range),
            related: diagnostic
                .related
                .into_iter()
                .map(|related| WasmRelatedInformation {
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
                        .map(|edit| WasmTextEdit {
                            range: wasm_text_range(edit.range),
                            replacement: edit.replacement.clone(),
                        })
                        .collect();
                    WasmFix {
                        title: fix.title,
                        applicability: match fix.applicability {
                            Applicability::Always => WasmApplicability::Always,
                            Applicability::Maybe => WasmApplicability::Maybe,
                        },
                        edits,
                    }
                })
                .collect(),
        })
        .collect()
}

pub(crate) fn wasm_document_symbols(symbols: Vec<DocumentSymbol>) -> Vec<WasmDocumentSymbol> {
    symbols.into_iter().map(wasm_document_symbol).collect()
}

fn wasm_document_symbol(symbol: DocumentSymbol) -> WasmDocumentSymbol {
    WasmDocumentSymbol {
        name: symbol.name,
        kind: match symbol.kind {
            SymbolKind::DocumentTitle => WasmSymbolKind::DocumentTitle,
            SymbolKind::Part => WasmSymbolKind::Part,
            SymbolKind::Section => WasmSymbolKind::Section,
            SymbolKind::ListItem => WasmSymbolKind::ListItem,
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

pub(crate) fn wasm_document_projection(doc: DocumentProjection) -> WasmDocumentProjection {
    let headings = doc
        .structure
        .headings()
        .iter()
        .map(|heading| {
            let presentation = doc
                .presentation
                .heading_at(heading.range)
                .expect("every projected heading has presentation facts");
            WasmStructuredHeading {
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
    let toc = doc.presentation.toc().iter().map(wasm_toc_entry).collect();
    let manpage = doc.structure.manpage().map(|manpage| WasmManpage {
        name: manpage.name.clone(),
        section: manpage.section.clone(),
        purpose: manpage.purpose.clone(),
        title_range: wasm_text_range(manpage.title_range),
        name_range: wasm_text_range(manpage.name_range),
        purpose_range: wasm_text_range(manpage.purpose_range),
    });
    let footnotes = doc
        .catalogs
        .footnotes()
        .iter()
        .map(|footnote| WasmFootnote {
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
    let bibliography = doc
        .catalogs
        .bibliography()
        .iter()
        .map(|entry| WasmBibliographyEntry {
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
    let index = doc
        .catalogs
        .index()
        .iter()
        .map(|entry| WasmIndexEntry {
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

    WasmDocumentProjection {
        source_id: doc.source_id.map(|source_id| source_id.as_str().to_owned()),
        source_blocks: doc
            .source_blocks
            .into_iter()
            .map(|source| WasmSourceBlockProjection {
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
        formulas: doc
            .formulas
            .into_iter()
            .map(|formula| WasmFormulaProjection {
                kind: match formula.kind {
                    FormulaKind::Inline => WasmFormulaKind::Inline,
                    FormulaKind::Block => WasmFormulaKind::Block,
                },
                language: math_language(formula.language),
                source_range: wasm_text_range(formula.source_range),
                content_range: wasm_text_range(formula.content_range),
                source: formula.source,
            })
            .collect(),
        citations: doc
            .citations
            .into_iter()
            .map(|citation| WasmCitationProjection {
                order: citation.order,
                source_range: wasm_text_range(citation.range),
                keys: citation
                    .keys
                    .into_iter()
                    .map(|key| WasmCitationKeyProjection {
                        source_range: wasm_text_range(key.range),
                        key: key.value,
                    })
                    .collect(),
                attributes: citation
                    .attributes
                    .into_iter()
                    .map(|attribute| WasmCitationAttributeProjection {
                        source_range: wasm_text_range(attribute.range),
                        name: attribute.name,
                        value: attribute.value,
                    })
                    .collect(),
            })
            .collect(),
        block_presentations: doc
            .block_presentations
            .into_iter()
            .map(|block| WasmBlockPresentationProjection {
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
        ordered_lists: doc
            .ordered_lists
            .into_iter()
            .map(|list| WasmOrderedListProjection {
                source_range: wasm_text_range(list.source_range),
                start: list.start,
                reversed: list.reversed,
                style: ordered_list_style(list.style),
            })
            .collect(),
        reference_edges: doc
            .reference_edges
            .into_iter()
            .map(|edge| WasmReferenceEdge {
                source_id: edge
                    .source_id
                    .map(|source_id| source_id.as_str().to_owned()),
                source_range: wasm_text_range(edge.source_range),
                target: reference_key(edge.target),
                resolution: edge.resolution.map(resolution_outcome),
            })
            .collect(),
        external_links: doc
            .external_links
            .into_iter()
            .map(|link| WasmExternalLink {
                source_range: wasm_text_range(link.source_range),
                target_range: wasm_text_range(link.target_range),
                target: link.target,
                label: link.label,
            })
            .collect(),
        searchable_text: WasmSearchableText {
            text: doc.searchable_text.text,
            segments: doc
                .searchable_text
                .segments
                .into_iter()
                .map(|segment| WasmSearchTextSegment {
                    kind: match segment.kind {
                        SearchTextKind::Prose => WasmSearchTextKind::Prose,
                        SearchTextKind::Code => WasmSearchTextKind::Code,
                    },
                    source_range: wasm_text_range(segment.source_range),
                    text: segment.text,
                })
                .collect(),
        },
        structure: WasmDocumentStructure {
            headings,
            toc,
            manpage,
        },
        catalogs: WasmDocumentCatalogs {
            footnotes,
            bibliography,
            index,
        },
        targets: doc
            .targets
            .into_iter()
            .map(|target| WasmReferenceTarget {
                kind: reference_target_kind(target.kind),
                id: target.id,
                label: target.label,
                id_range: wasm_text_range(target.id_range),
                target_range: wasm_text_range(target.target_range),
            })
            .collect(),
        title: doc.title.map(wasm_projected_text),
    }
}

fn wasm_projected_text(text: ProjectedText) -> WasmProjectedText {
    WasmProjectedText {
        source_range: wasm_text_range(text.source_range),
        text: text.text,
    }
}

fn wasm_toc_entry(entry: &TocEntry) -> WasmTocEntry {
    WasmTocEntry {
        id: entry.id.clone(),
        title: entry.title.clone(),
        level: u32::from(entry.level),
        number: entry.number.clone(),
        range: wasm_text_range(entry.range),
        children: entry.children.iter().map(wasm_toc_entry).collect(),
    }
}

const fn math_language(language: MathLanguage) -> WasmMathLanguage {
    match language {
        MathLanguage::Latex => WasmMathLanguage::Latex,
        MathLanguage::Typst => WasmMathLanguage::Typst,
    }
}

const fn section_kind(kind: SectionKind) -> WasmSectionKind {
    match kind {
        SectionKind::DocumentTitle => WasmSectionKind::DocumentTitle,
        SectionKind::Part => WasmSectionKind::Part,
        SectionKind::Section => WasmSectionKind::Section,
        SectionKind::Appendix => WasmSectionKind::Appendix,
        SectionKind::Discrete => WasmSectionKind::Discrete,
    }
}

const fn reference_target_kind(kind: ReferenceTargetKind) -> WasmReferenceTargetKind {
    match kind {
        ReferenceTargetKind::DocumentTitle => WasmReferenceTargetKind::DocumentTitle,
        ReferenceTargetKind::Part => WasmReferenceTargetKind::Part,
        ReferenceTargetKind::Section => WasmReferenceTargetKind::Section,
        ReferenceTargetKind::ExplicitAnchor => WasmReferenceTargetKind::ExplicitAnchor,
        ReferenceTargetKind::InlineAnchor => WasmReferenceTargetKind::InlineAnchor,
    }
}

const fn block_presentation_kind(kind: BlockPresentationKind) -> WasmBlockPresentationKind {
    match kind {
        BlockPresentationKind::Admonition => WasmBlockPresentationKind::Admonition,
        BlockPresentationKind::Quote => WasmBlockPresentationKind::Quote,
        BlockPresentationKind::Verse => WasmBlockPresentationKind::Verse,
        BlockPresentationKind::Example => WasmBlockPresentationKind::Example,
        BlockPresentationKind::Sidebar => WasmBlockPresentationKind::Sidebar,
        BlockPresentationKind::Open => WasmBlockPresentationKind::Open,
        BlockPresentationKind::Collapsible => WasmBlockPresentationKind::Collapsible,
        BlockPresentationKind::Figure => WasmBlockPresentationKind::Figure,
        BlockPresentationKind::Table => WasmBlockPresentationKind::Table,
    }
}

const fn ordered_list_style(style: OrderedListStyle) -> WasmOrderedListStyle {
    match style {
        OrderedListStyle::Arabic => WasmOrderedListStyle::Arabic,
        OrderedListStyle::Decimal => WasmOrderedListStyle::Decimal,
        OrderedListStyle::LowerAlpha => WasmOrderedListStyle::Loweralpha,
        OrderedListStyle::UpperAlpha => WasmOrderedListStyle::Upperalpha,
        OrderedListStyle::LowerRoman => WasmOrderedListStyle::Lowerroman,
        OrderedListStyle::UpperRoman => WasmOrderedListStyle::Upperroman,
        OrderedListStyle::LowerGreek => WasmOrderedListStyle::Lowergreek,
    }
}

fn reference_key(key: ReferenceKey) -> WasmReferenceKey {
    match key {
        ReferenceKey::Local { anchor } => WasmReferenceKey::Local { anchor },
        ReferenceKey::Document { document, anchor } => {
            WasmReferenceKey::Document { document, anchor }
        }
        ReferenceKey::Scheme {
            scheme,
            locator,
            anchor,
        } => WasmReferenceKey::Scheme {
            scheme,
            locator,
            anchor,
        },
    }
}

fn resolution_outcome(outcome: ResolutionOutcome) -> WasmProjectedResolutionOutcome {
    match outcome {
        ResolutionOutcome::Resolved {
            href,
            display_text,
            notices,
        } => WasmProjectedResolutionOutcome::Resolved {
            href,
            display_text,
            notices: notices
                .into_iter()
                .map(|notice| match notice.kind {
                    ResolutionNoticeKind::Fallback => {
                        WasmProjectedReferenceNotice::ReferenceResolutionFallback
                    }
                })
                .collect(),
        },
        ResolutionOutcome::Failed(failure) => WasmProjectedResolutionOutcome::Failed {
            kind: match failure.kind {
                ResolutionFailureKind::MissingTarget => {
                    WasmProjectedReferenceFailureKind::MissingReferenceTarget
                }
                ResolutionFailureKind::MissingAnchor => {
                    WasmProjectedReferenceFailureKind::MissingReferenceAnchor
                }
                ResolutionFailureKind::AmbiguousTarget => {
                    WasmProjectedReferenceFailureKind::AmbiguousReferenceTarget
                }
                ResolutionFailureKind::OutsideRoot => {
                    WasmProjectedReferenceFailureKind::ReferenceOutsideRoot
                }
                ResolutionFailureKind::ResolverFailure => {
                    WasmProjectedReferenceFailureKind::ReferenceResolverFailure
                }
            },
        },
    }
}

pub(crate) fn wasm_text_range(range: adocweave::text::TextRange) -> WasmTextRange {
    WasmTextRange {
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
            (MathLanguage::Latex, WasmMathLanguage::Latex),
            (MathLanguage::Typst, WasmMathLanguage::Typst),
        ] {
            assert_eq!(math_language(core), wire);
        }
        for (core, wire) in [
            (SectionKind::DocumentTitle, WasmSectionKind::DocumentTitle),
            (SectionKind::Part, WasmSectionKind::Part),
            (SectionKind::Section, WasmSectionKind::Section),
            (SectionKind::Appendix, WasmSectionKind::Appendix),
            (SectionKind::Discrete, WasmSectionKind::Discrete),
        ] {
            assert_eq!(section_kind(core), wire);
        }
        for (core, wire) in [
            (
                ReferenceTargetKind::DocumentTitle,
                WasmReferenceTargetKind::DocumentTitle,
            ),
            (ReferenceTargetKind::Part, WasmReferenceTargetKind::Part),
            (
                ReferenceTargetKind::Section,
                WasmReferenceTargetKind::Section,
            ),
            (
                ReferenceTargetKind::ExplicitAnchor,
                WasmReferenceTargetKind::ExplicitAnchor,
            ),
            (
                ReferenceTargetKind::InlineAnchor,
                WasmReferenceTargetKind::InlineAnchor,
            ),
        ] {
            assert_eq!(reference_target_kind(core), wire);
        }
        for (core, wire) in [
            (
                BlockPresentationKind::Admonition,
                WasmBlockPresentationKind::Admonition,
            ),
            (
                BlockPresentationKind::Quote,
                WasmBlockPresentationKind::Quote,
            ),
            (
                BlockPresentationKind::Verse,
                WasmBlockPresentationKind::Verse,
            ),
            (
                BlockPresentationKind::Example,
                WasmBlockPresentationKind::Example,
            ),
            (
                BlockPresentationKind::Sidebar,
                WasmBlockPresentationKind::Sidebar,
            ),
            (BlockPresentationKind::Open, WasmBlockPresentationKind::Open),
            (
                BlockPresentationKind::Collapsible,
                WasmBlockPresentationKind::Collapsible,
            ),
            (
                BlockPresentationKind::Figure,
                WasmBlockPresentationKind::Figure,
            ),
            (
                BlockPresentationKind::Table,
                WasmBlockPresentationKind::Table,
            ),
        ] {
            assert_eq!(block_presentation_kind(core), wire);
        }
        for (core, wire) in [
            (OrderedListStyle::Arabic, WasmOrderedListStyle::Arabic),
            (OrderedListStyle::Decimal, WasmOrderedListStyle::Decimal),
            (
                OrderedListStyle::LowerAlpha,
                WasmOrderedListStyle::Loweralpha,
            ),
            (
                OrderedListStyle::UpperAlpha,
                WasmOrderedListStyle::Upperalpha,
            ),
            (
                OrderedListStyle::LowerRoman,
                WasmOrderedListStyle::Lowerroman,
            ),
            (
                OrderedListStyle::UpperRoman,
                WasmOrderedListStyle::Upperroman,
            ),
            (
                OrderedListStyle::LowerGreek,
                WasmOrderedListStyle::Lowergreek,
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
                WasmProjectedReferenceFailureKind::MissingReferenceTarget,
            ),
            (
                ResolutionFailureKind::MissingAnchor,
                WasmProjectedReferenceFailureKind::MissingReferenceAnchor,
            ),
            (
                ResolutionFailureKind::AmbiguousTarget,
                WasmProjectedReferenceFailureKind::AmbiguousReferenceTarget,
            ),
            (
                ResolutionFailureKind::OutsideRoot,
                WasmProjectedReferenceFailureKind::ReferenceOutsideRoot,
            ),
            (
                ResolutionFailureKind::ResolverFailure,
                WasmProjectedReferenceFailureKind::ReferenceResolverFailure,
            ),
        ] {
            assert_eq!(
                resolution_outcome(ResolutionOutcome::Failed(ResolverFailure { kind: core })),
                WasmProjectedResolutionOutcome::Failed { kind: wire },
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
            WasmProjectedResolutionOutcome::Resolved {
                href: "#target".to_owned(),
                display_text: Some("Target".to_owned()),
                notices: vec![WasmProjectedReferenceNotice::ReferenceResolutionFallback],
            },
        );
    }
}
