//! Selected source content, with the only reuse exception restricted to footnotes.

use super::{HostError, HostResult, Selection, protocol::Scopes};
use adocweave_core::{
    Analysis,
    output::projection::{FormulaKind, FormulaProjection, formulas},
    semantic::{Citation, CitationKey, Inline, SemanticNode, StandardMacroKind, walk_inlines},
    text::TextRange,
};
use std::collections::BTreeSet;

#[derive(Default)]
pub struct SelectedContent {
    pub equations: Vec<FormulaProjection>,
    pub citations: Vec<Citation>,
    pub stem_ranges: BTreeSet<TextRange>,
}

/// Adds a referenced footnote at its first occurrence in each fixed scope.
/// Public callers exclude notes before looking up definitions or dependencies.
pub fn selected_content(
    analysis: &Analysis,
    body: &Selection,
    notes: &Selection,
    include_notes: bool,
) -> HostResult<Scopes<SelectedContent>> {
    let empty = Selection::default();
    let notes = if include_notes { notes } else { &empty };
    let projections = formulas(analysis);
    let citations = analysis.citations();
    let mut ownership = BTreeSet::new();
    for selection in [body, notes] {
        let mut unique = BTreeSet::new();
        for range in selection
            .equations
            .iter()
            .chain(&selection.citations)
            .chain(&selection.footnotes)
        {
            if !unique.insert(*range) || !ownership.insert(*range) {
                return Err(HostError::protocol("source selected more than once").at(Some(*range)));
            }
        }
    }
    let collect = |selection: &Selection, allow_body: bool| -> HostResult<SelectedContent> {
        let mut equations = Vec::new();
        let mut cited = Vec::new();
        let mut stem_ranges = BTreeSet::new();
        for range in &selection.equations {
            let formula = projections
                .iter()
                .find(|p| p.source_range == *range)
                .ok_or_else(|| {
                    HostError::protocol("unknown equation source range").at(Some(*range))
                })?;
            equations.push(((*range, *range), formula.clone()));
        }
        for range in &selection.citations {
            let citation = citations
                .iter()
                .find(|p| p.range == *range)
                .ok_or_else(|| {
                    HostError::protocol("unknown citation source range").at(Some(*range))
                })?;
            cited.push(((*range, *range), citation.clone()));
        }
        let mut occurrences = selection.footnotes.clone();
        occurrences.sort_unstable();
        let mut definitions = BTreeSet::new();
        for range in occurrences {
            let (footnote, _) = analysis
                .document()
                .catalogs()
                .footnote_occurrence(range)
                .ok_or_else(|| {
                    HostError::protocol("unknown footnote reference range").at(Some(range))
                })?;
            if !selection.footnotes.contains(&footnote.definition_range)
                && !(allow_body && body.footnotes.contains(&footnote.definition_range))
            {
                return Err(HostError::new(
                    "slides-footnote-outside-scope",
                    "footnote definition is outside the allowed slide scope",
                )
                .at(Some(range)));
            }
            if !definitions.insert(footnote.definition_range) {
                continue;
            }
            let inlines = analysis
                .document()
                .footnote_body(footnote.definition_range)
                .ok_or_else(|| HostError::protocol("footnote body is missing").at(Some(range)))?;
            let mut unsupported_anchor = None;
            walk_inlines(inlines, |node| match node {
                SemanticNode::Inline(Inline::Macro(node))
                    if matches!(
                        node.kind,
                        StandardMacroKind::Anchor | StandardMacroKind::BibliographyAnchor
                    ) =>
                {
                    unsupported_anchor.get_or_insert(node.range);
                }
                SemanticNode::Inline(Inline::Formula(formula)) => {
                    if formula.uses_stem_attribute {
                        stem_ranges.insert(formula.range);
                    }
                    equations.push((
                        (range, formula.range),
                        FormulaProjection {
                            kind: FormulaKind::Inline,
                            language: formula.language,
                            source_range: formula.range,
                            content_range: formula.content_range,
                            source: formula.value.clone(),
                        },
                    ));
                }
                SemanticNode::Inline(Inline::Macro(node))
                    if node.kind == StandardMacroKind::Citation =>
                {
                    cited.push((
                        (range, node.range),
                        Citation {
                            range: node.range,
                            order: 0,
                            keys: node
                                .attributes
                                .iter()
                                .filter(|a| a.name.is_none())
                                .map(|a| CitationKey {
                                    range: a.value_range,
                                    value: a.value.clone(),
                                })
                                .collect(),
                            attributes: node
                                .attributes
                                .iter()
                                .filter(|a| a.name.is_some())
                                .cloned()
                                .collect(),
                        },
                    ));
                }
                _ => {}
            });
            if let Some(range) = unsupported_anchor {
                return Err(HostError::new(
                    "slides-footnote-anchor-unsupported",
                    "anchor and bibliography definitions inside slide footnotes are unsupported",
                )
                .at(Some(range)));
            }
        }
        equations.sort_unstable_by_key(|(order, _)| *order);
        cited.sort_unstable_by_key(|(order, _)| *order);
        Ok(SelectedContent {
            equations: equations.into_iter().map(|(_, value)| value).collect(),
            citations: cited
                .into_iter()
                .enumerate()
                .map(|(order, (_, mut value))| {
                    value.order = order as u32;
                    value
                })
                .collect(),
            stem_ranges,
        })
    };
    Ok(Scopes {
        body: collect(body, false)?,
        notes: collect(notes, true)?,
    })
}
