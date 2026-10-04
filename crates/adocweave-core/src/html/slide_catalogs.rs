//! Output-only numbering and landing points for the two slide scopes.

use std::collections::{BTreeMap, BTreeSet};

use crate::caption::{BlockCaption, CaptionFamily};
use crate::catalog::Footnote;
use crate::document::Document;
use crate::generated_bibliography::BibliographyNamespace;
use crate::inline_model::{Inline, StandardMacroKind};
use crate::render::RenderInputs;
use crate::source::TextRange;
use crate::walker::{self, SemanticNode};

use super::body::{self, BlockWriter, classes, passive};
use super::regions::{HtmlRegionError, HtmlRegionSelection};
use super::{InlineRenderContext, safe};

/// The two independent numbering and bibliography scopes in a slide deck.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlSlideScope {
    Body,
    Notes,
}

#[cfg(test)]
mod tests;

impl HtmlSlideScope {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Notes => "notes",
        }
    }

    pub(super) const fn namespace(self) -> BibliographyNamespace {
        match self {
            Self::Body => BibliographyNamespace::SlidesBody,
            Self::Notes => BibliographyNamespace::SlidesNotes,
        }
    }

    /// Fixed container ID for the single bibliography placement in this scope.
    pub const fn bibliography_id(self) -> &'static str {
        match self {
            Self::Body => "slides-body-references",
            Self::Notes => "slides-notes-references",
        }
    }
}

/// Regions grouped by real source slide, including a heading and any columns.
///
/// Notes may reuse a footnote defined in a selected body region. Body regions
/// may only use definitions selected in the body. Public hosts leave `notes`
/// empty and render only `Body`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HtmlSlideSelections {
    pub body: Vec<Vec<HtmlRegionSelection>>,
    pub notes: Vec<Vec<HtmlRegionSelection>>,
}

impl HtmlSlideSelections {
    pub(super) fn groups(&self, scope: HtmlSlideScope) -> &[Vec<HtmlRegionSelection>] {
        match scope {
            HtmlSlideScope::Body => &self.body,
            HtmlSlideScope::Notes => &self.notes,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct FootnoteLink {
    pub(super) number: u32,
    pub(super) reference_id: String,
    pub(super) target_id: String,
}

#[derive(Debug)]
struct FootnotePlacement<'document> {
    footnote: &'document Footnote,
    number: u32,
    target_id: String,
    reference_ids: Vec<String>,
}

#[derive(Debug)]
pub(super) struct SlideCatalogs<'document> {
    scope: HtmlSlideScope,
    captions: BTreeMap<TextRange, BlockCaption>,
    shared_body_captions: BTreeMap<TextRange, BlockCaption>,
    shared_body_ranges: BTreeSet<TextRange>,
    footnote_links: BTreeMap<(usize, TextRange), FootnoteLink>,
    footnotes: Vec<Vec<FootnotePlacement<'document>>>,
    pub(super) selected: BTreeSet<TextRange>,
    pub(super) generated_ids: BTreeSet<String>,
}

fn walk_selected<'document>(
    document: &'document Document,
    selection: &HtmlRegionSelection,
    mut visitor: impl FnMut(SemanticNode<'document>),
) -> Result<(), HtmlRegionError> {
    super::regions::RegionPresentation::prepare(document, selection)?;
    fn visit<'document>(
        document: &'document Document,
        selection: &HtmlRegionSelection,
        node: SemanticNode<'document>,
        visitor: &mut impl FnMut(SemanticNode<'document>),
    ) {
        if let SemanticNode::Block(block) = node
            && selection.omitted_blocks.contains(
                &document
                    .index()
                    .block_id_at(block.range())
                    .expect("document block is indexed"),
            )
        {
            return;
        }
        visitor(node);
        let _: std::ops::ControlFlow<()> = walker::try_visit_children(node, &mut |child| {
            visit(document, selection, child, visitor);
            std::ops::ControlFlow::Continue(())
        });
    }
    for &id in &selection.blocks {
        let block = document
            .block(id)
            .ok_or(HtmlRegionError::InvalidSelection {
                block: id,
                reason: "block does not belong to this document",
            })?;
        visit(
            document,
            selection,
            SemanticNode::Block(block),
            &mut visitor,
        );
    }
    Ok(())
}

fn node_range(node: SemanticNode<'_>) -> Option<TextRange> {
    match node {
        SemanticNode::Block(block) => Some(block.range()),
        SemanticNode::Inline(inline) => Some(inline.range()),
        _ => None,
    }
}

pub(super) fn reference_id(scope: HtmlSlideScope, range: TextRange) -> String {
    format!("slides-{}-bib-ref-{}", scope.name(), range.start().to_u32())
}

impl<'document> SlideCatalogs<'document> {
    pub(super) fn prepare(
        document: &'document Document,
        inputs: &RenderInputs,
        selections: &HtmlSlideSelections,
        scope: HtmlSlideScope,
        reserved_ids: &BTreeSet<String>,
    ) -> Result<Self, HtmlRegionError> {
        let groups = selections.groups(scope);
        let mut selected = BTreeSet::new();
        let mut group_footnotes = Vec::with_capacity(groups.len());
        for group in groups {
            let mut occurrences = Vec::new();
            for selection in group {
                walk_selected(document, selection, |node| {
                    let Some(range) = node_range(node) else {
                        return;
                    };
                    selected.insert(range);
                    if let SemanticNode::Inline(Inline::Macro(node)) = node
                        && node.kind == StandardMacroKind::Citation
                    {
                        selected.extend(
                            node.attributes
                                .iter()
                                .filter(|key| key.name.is_none())
                                .map(|key| key.value_range),
                        );
                    }
                    if let SemanticNode::Inline(Inline::Macro(node)) = node
                        && node.kind == StandardMacroKind::Footnote
                        && let Some((footnote, _)) = document.catalogs().footnote_occurrence(range)
                    {
                        occurrences.push((range, footnote));
                    }
                })?;
            }
            occurrences.sort_unstable_by_key(|(range, _)| *range);
            group_footnotes.push(occurrences);
        }
        let mut plan = Self {
            scope,
            captions: BTreeMap::new(),
            shared_body_captions: BTreeMap::new(),
            shared_body_ranges: BTreeSet::new(),
            footnote_links: BTreeMap::new(),
            footnotes: Vec::with_capacity(groups.len()),
            selected,
            generated_ids: BTreeSet::new(),
        };
        if scope == HtmlSlideScope::Notes {
            for selection in selections.body.iter().flatten() {
                walk_selected(document, selection, |node| {
                    if let Some(range) = node_range(node) {
                        plan.shared_body_ranges.insert(range);
                    }
                })?;
            }
        }
        let mut occupied = reserved_ids.clone();
        occupied.extend(
            document
                .reference_targets()
                .iter()
                .map(|target| target.id.clone()),
        );
        for math in inputs.math() {
            for id in math.value().ids() {
                if !occupied.insert(id.clone()) {
                    return Err(HtmlRegionError::GeneratedIdCollision {
                        id: id.clone(),
                        range: math.source_range,
                    });
                }
                plan.generated_ids.insert(id.clone());
            }
        }
        let mut caption_numbers = [0u32; 4];
        let mut body_caption_numbers = [0u32; 4];
        for caption in document.presentation().captions() {
            if plan.shared_body_ranges.contains(&caption.range) {
                let displayed = renumber_caption(caption, &mut body_caption_numbers);
                plan.shared_body_captions.insert(caption.range, displayed);
            }
            if !plan.selected.contains(&caption.range) {
                continue;
            }
            let displayed = renumber_caption(caption, &mut caption_numbers);
            plan.captions.insert(caption.range, displayed);
        }
        for reference in document.inner().facts().references() {
            if plan.selected.contains(&reference.range)
                && let Some(crate::reference::ReferenceKey::Local { anchor }) = &reference.target
                && let Some(target) = document.inner().identifiers().target_by_id(anchor)
                && !plan.permits_target(target.target_range)
            {
                return Err(HtmlRegionError::ReferenceOutsideScope {
                    range: reference.target_range,
                });
            }
        }
        let mut footnote_numbers = BTreeMap::new();
        for (slide, occurrences) in group_footnotes.into_iter().enumerate() {
            let mut placements = Vec::<FootnotePlacement<'document>>::new();
            let mut placement_by_number = BTreeMap::new();
            for (range, footnote) in occurrences {
                let definition_selected = plan.selected.contains(&footnote.definition_range)
                    || plan.shared_body_ranges.contains(&footnote.definition_range);
                if !definition_selected {
                    return Err(HtmlRegionError::FootnoteOutsideScope { range });
                }
                let next = u32::try_from(footnote_numbers.len() + 1)
                    .expect("footnotes are bounded by the document node limit");
                let number = *footnote_numbers.entry(footnote.number).or_insert(next);
                let target_id = format!("slides-{}-s{}-footnote-{number}", scope.name(), slide + 1);
                let reference_id = format!(
                    "slides-{}-s{}-footnote-ref-{}",
                    scope.name(),
                    slide + 1,
                    range.start().to_u32()
                );
                let placement = match placement_by_number.entry(number) {
                    std::collections::btree_map::Entry::Occupied(entry) => *entry.get(),
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        plan.insert_id(&mut occupied, &target_id, range)?;
                        let placement = placements.len();
                        entry.insert(placement);
                        placements.push(FootnotePlacement {
                            footnote,
                            number,
                            target_id: target_id.clone(),
                            reference_ids: Vec::new(),
                        });
                        placement
                    }
                };
                plan.insert_id(&mut occupied, &reference_id, range)?;
                placements[placement]
                    .reference_ids
                    .push(reference_id.clone());
                plan.footnote_links.insert(
                    (slide, range),
                    FootnoteLink {
                        number,
                        reference_id,
                        target_id,
                    },
                );
            }
            placements.sort_unstable_by_key(|placement| placement.number);
            plan.footnotes.push(placements);
        }
        if let Some(bibliography) = inputs.generated_bibliography() {
            if bibliography.namespace() != scope.namespace() {
                return Err(HtmlRegionError::InvalidBibliographyScope { scope });
            }
            if !bibliography.entries().is_empty() {
                let zero =
                    TextRange::new(crate::source::TextSize::ZERO, crate::source::TextSize::ZERO)
                        .expect("zero range is ordered");
                plan.insert_id(&mut occupied, scope.bibliography_id(), zero)?;
                for entry in bibliography.entries() {
                    let anchor = bibliography.namespace().anchor_id(entry.citation_key());
                    plan.insert_id(&mut occupied, &anchor, zero)?;
                }
            }
        }
        for node in document.inner().facts().macros() {
            if node.kind != StandardMacroKind::Citation || !plan.selected.contains(&node.range) {
                continue;
            }
            for key in node.attributes.iter().filter(|key| key.name.is_none()) {
                let generated = inputs.generated_bibliography().is_some_and(|bibliography| {
                    bibliography
                        .entries()
                        .iter()
                        .any(|entry| entry.citation_key() == key.value)
                });
                let authored = document
                    .catalogs()
                    .bibliography()
                    .iter()
                    .find(|entry| entry.id == key.value);
                if !generated
                    && authored.is_some_and(|entry| !plan.permits_target(entry.definition_range))
                {
                    return Err(HtmlRegionError::ReferenceOutsideScope {
                        range: key.value_range,
                    });
                }
                let defined = generated || authored.is_some();
                if defined {
                    plan.insert_id(
                        &mut occupied,
                        &reference_id(scope, key.value_range),
                        key.value_range,
                    )?;
                }
            }
        }
        for entry in document.catalogs().bibliography() {
            for reference in &entry.references {
                if plan.selected.contains(&reference.range) {
                    // Citation keys have already registered their landing point.
                    let id = reference_id(scope, reference.range);
                    if !plan.generated_ids.contains(&id) {
                        plan.insert_id(&mut occupied, &id, reference.range)?;
                    }
                }
            }
        }
        Ok(plan)
    }

    fn insert_id(
        &mut self,
        occupied: &mut BTreeSet<String>,
        id: &str,
        range: TextRange,
    ) -> Result<(), HtmlRegionError> {
        if !occupied.insert(id.to_owned()) {
            return Err(HtmlRegionError::GeneratedIdCollision {
                id: id.to_owned(),
                range,
            });
        }
        self.generated_ids.insert(id.to_owned());
        Ok(())
    }

    pub(super) fn caption(&self, range: TextRange) -> Option<&BlockCaption> {
        self.captions.get(&range)
    }

    pub(super) fn reference_caption(&self, range: TextRange) -> Option<&BlockCaption> {
        self.captions
            .get(&range)
            .or_else(|| self.shared_body_captions.get(&range))
    }

    pub(super) fn permits_target(&self, range: TextRange) -> bool {
        self.selected.contains(&range) || self.shared_body_ranges.contains(&range)
    }

    pub(super) fn footnote_link(&self, slide: usize, range: TextRange) -> Option<&FootnoteLink> {
        self.footnote_links.get(&(slide, range))
    }

    pub(super) fn reference_id(&self, range: TextRange) -> String {
        reference_id(self.scope, range)
    }

    pub(super) fn render_footnotes(
        &self,
        slide: usize,
        document: &Document,
        context: &mut InlineRenderContext<'_, '_>,
        prior_bytes: usize,
        limits: crate::OutputLimits,
    ) -> Result<String, HtmlRegionError> {
        let mut output = String::new();
        if self.footnotes[slide].is_empty() {
            return Ok(output);
        }
        BlockWriter::start(&mut output, "div", &[classes(&["footnotes"])]);
        BlockWriter::line_break(&mut output);
        BlockWriter::start(&mut output, "ol", &[]);
        BlockWriter::line_break(&mut output);
        for placement in &self.footnotes[slide] {
            BlockWriter::start(
                &mut output,
                "li",
                &[
                    passive("id", &placement.target_id),
                    passive("value", placement.number.to_string()),
                ],
            );
            if let Some(inlines) = document
                .inner()
                .facts()
                .footnote_body(placement.footnote.definition_range)
            {
                let inlines = body::plan_inlines(inlines, context);
                body::serialize_inlines(&mut output, &inlines);
            } else {
                BlockWriter::inline_text(&mut output, &placement.footnote.text);
            }
            for reference_id in &placement.reference_ids {
                let href = safe::SafeFragmentUrl::new(reference_id)
                    .expect("generated footnote ID is safe")
                    .into_owned();
                BlockWriter::text(&mut output, " ");
                BlockWriter::start(
                    &mut output,
                    "a",
                    &[
                        classes(&["footnote-backref"]),
                        body::fragment_url("href", href),
                    ],
                );
                BlockWriter::text(&mut output, "↩");
                BlockWriter::end(&mut output, "a");
                super::regions::check_output_limit(prior_bytes, output.len(), limits)?;
            }
            BlockWriter::end(&mut output, "li");
            BlockWriter::line_break(&mut output);
            super::regions::check_output_limit(prior_bytes, output.len(), limits)?;
        }
        BlockWriter::end(&mut output, "ol");
        BlockWriter::line_break(&mut output);
        BlockWriter::end(&mut output, "div");
        BlockWriter::line_break(&mut output);
        super::regions::check_output_limit(prior_bytes, output.len(), limits)?;
        Ok(output)
    }
}

fn renumber_caption(caption: &BlockCaption, counters: &mut [u32; 4]) -> BlockCaption {
    let mut displayed = caption.clone();
    if caption.number.is_some() {
        let family = match caption.family {
            CaptionFamily::Figure => 0,
            CaptionFamily::Table => 1,
            CaptionFamily::Example => 2,
            CaptionFamily::Listing => 3,
        };
        counters[family] += 1;
        displayed.number = Some(counters[family]);
    }
    displayed
}
