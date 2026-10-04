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
    body_container_headings: BTreeSet<TextRange>,
    footnote_links: BTreeMap<(usize, TextRange), FootnoteLink>,
    footnotes: Vec<Vec<FootnotePlacement<'document>>>,
    math: BTreeMap<(Option<(usize, TextRange)>, TextRange), crate::rendered_content::ValidatedMath>,
    citation_references: BTreeMap<String, Vec<String>>,
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
        limits: crate::OutputLimits,
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
            body_container_headings: BTreeSet::new(),
            footnote_links: BTreeMap::new(),
            footnotes: Vec::with_capacity(groups.len()),
            math: BTreeMap::new(),
            citation_references: BTreeMap::new(),
            selected,
            generated_ids: BTreeSet::new(),
        };
        for selection in selections.body.iter().flatten() {
            for &id in &selection.container_headings {
                let Some(crate::block_model::AstBlock::Heading(heading)) = document.block(id)
                else {
                    return Err(HtmlRegionError::InvalidSelection {
                        block: id,
                        reason: "container heading is not a heading",
                    });
                };
                plan.body_container_headings.insert(heading.range);
            }
        }
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
        let mut footnote_equations = BTreeMap::<TextRange, BTreeSet<TextRange>>::new();
        let mut footnote_citations = Vec::new();
        for (slide, placements) in plan.footnotes.iter().enumerate() {
            for placement in placements {
                let definition = placement.footnote.definition_range;
                if let Some(inlines) = document.footnote_body(definition) {
                    walker::walk_inlines(inlines, |node| {
                        if let Some(range) = node_range(node) {
                            plan.selected.insert(range);
                        }
                        match node {
                            SemanticNode::Inline(Inline::Formula(formula)) => {
                                footnote_equations
                                    .entry(definition)
                                    .or_default()
                                    .insert(formula.range);
                            }
                            SemanticNode::Inline(Inline::Macro(node))
                                if node.kind == StandardMacroKind::Citation =>
                            {
                                footnote_citations.push((slide, definition, node));
                            }
                            _ => {}
                        }
                    });
                }
            }
        }
        plan.prepare_math(inputs, &footnote_equations, &mut occupied, limits)?;
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
        let main_citations = document
            .inner()
            .facts()
            .macros()
            .iter()
            .filter(|node| {
                node.kind == StandardMacroKind::Citation && plan.selected.contains(&node.range)
            })
            .map(|node| (None, node))
            .collect::<Vec<_>>();
        let citations = main_citations.into_iter().chain(
            footnote_citations
                .into_iter()
                .map(|(slide, definition, node)| (Some((slide, definition)), node)),
        );
        for (placement, node) in citations {
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
                    let id = placement.map_or_else(
                        || reference_id(scope, key.value_range),
                        |(slide, definition)| {
                            plan.placement_reference_id(slide, Some(definition), key.value_range)
                        },
                    );
                    plan.insert_id(&mut occupied, &id, key.value_range)?;
                    plan.citation_references
                        .entry(key.value.clone())
                        .or_default()
                        .push(id);
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
                        plan.citation_references
                            .entry(entry.id.clone())
                            .or_default()
                            .push(id);
                    }
                }
            }
        }
        Ok(plan)
    }

    fn prepare_math(
        &mut self,
        inputs: &RenderInputs,
        footnote_equations: &BTreeMap<TextRange, BTreeSet<TextRange>>,
        occupied: &mut BTreeSet<String>,
        limits: crate::OutputLimits,
    ) -> Result<(), HtmlRegionError> {
        // First placements are canonical targets for equation references in ordinary prose.
        let mut canonical_ids = BTreeMap::new();
        let mut seen_definitions = BTreeSet::new();
        for placements in &self.footnotes {
            for placement in placements {
                if !seen_definitions.insert(placement.footnote.definition_range) {
                    continue;
                }
                if let Some(ranges) = footnote_equations.get(&placement.footnote.definition_range) {
                    for math in inputs
                        .math()
                        .iter()
                        .filter(|math| ranges.contains(&math.source_range))
                    {
                        for id in math.value().ids() {
                            canonical_ids
                                .insert(id.clone(), format!("{}-{id}", placement.target_id));
                        }
                    }
                }
            }
        }
        let mut math_bytes = 0usize;
        for math in inputs.math() {
            if footnote_equations
                .values()
                .any(|ranges| ranges.contains(&math.source_range))
            {
                continue;
            }
            let value = math.value().remap_ids(&canonical_ids);
            if self.selected.contains(&math.source_range) {
                super::regions::check_output_limit(
                    math_bytes,
                    value.svg().len() + value.mathml().len(),
                    limits,
                )?;
                math_bytes += value.svg().len() + value.mathml().len();
            }
            for id in value.ids() {
                self.insert_id(occupied, id, math.source_range)?;
            }
            self.math.insert((None, math.source_range), value);
        }
        for slide in 0..self.footnotes.len() {
            for index in 0..self.footnotes[slide].len() {
                let placement = &self.footnotes[slide][index];
                let definition = placement.footnote.definition_range;
                let target = placement.target_id.clone();
                let Some(ranges) = footnote_equations.get(&definition) else {
                    continue;
                };
                let mut mapping = canonical_ids.clone();
                for math in inputs
                    .math()
                    .iter()
                    .filter(|math| ranges.contains(&math.source_range))
                {
                    for id in math.value().ids() {
                        mapping.insert(id.clone(), format!("{target}-{id}"));
                    }
                }
                for math in inputs
                    .math()
                    .iter()
                    .filter(|math| ranges.contains(&math.source_range))
                {
                    let value = math.value().remap_ids(&mapping);
                    super::regions::check_output_limit(
                        math_bytes,
                        value.svg().len() + value.mathml().len(),
                        limits,
                    )?;
                    math_bytes += value.svg().len() + value.mathml().len();
                    for id in value.ids() {
                        self.insert_id(occupied, id, math.source_range)?;
                    }
                    self.math
                        .insert((Some((slide, definition)), math.source_range), value);
                }
            }
        }
        Ok(())
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
        self.selected.contains(&range)
            || self.shared_body_ranges.contains(&range)
            || self.body_container_headings.contains(&range)
    }

    pub(super) fn footnote_link(&self, slide: usize, range: TextRange) -> Option<&FootnoteLink> {
        self.footnote_links.get(&(slide, range))
    }

    pub(super) fn reference_id(&self, range: TextRange) -> String {
        reference_id(self.scope, range)
    }

    pub(super) fn placement_reference_id(
        &self,
        slide: usize,
        footnote: Option<TextRange>,
        range: TextRange,
    ) -> String {
        if let Some(definition) = footnote {
            let placement = self.footnotes[slide]
                .iter()
                .find(|p| p.footnote.definition_range == definition)
                .expect("selected footnote placement");
            format!("{}-bib-ref-{}", placement.target_id, range.start().to_u32())
        } else {
            self.reference_id(range)
        }
    }

    pub(super) fn bibliography_references(&self, key: &str) -> &[String] {
        self.citation_references.get(key).map_or(&[], Vec::as_slice)
    }

    pub(super) fn math_value(
        &self,
        slide: usize,
        footnote: Option<TextRange>,
        range: TextRange,
    ) -> Option<&crate::rendered_content::ValidatedMath> {
        self.math
            .get(&(footnote.map(|definition| (slide, definition)), range))
    }

    pub(super) fn bibliography_id(&self) -> &'static str {
        self.scope.bibliography_id()
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
            context.footnote = Some(placement.footnote.definition_range);
            if let Some(inlines) = document.footnote_body(placement.footnote.definition_range) {
                let inlines = body::plan_inlines(inlines, context);
                body::serialize_inlines(&mut output, &inlines);
            } else {
                BlockWriter::inline_text(&mut output, &placement.footnote.text);
            }
            context.footnote = None;
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
