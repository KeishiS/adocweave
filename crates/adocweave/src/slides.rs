//! Slide composition over the existing immutable semantic document.
//!
//! The deck owns only placement facts and block identities. Escaping and body
//! rendering remain in the core HTML writer; the CLI supplies trusted reveal
//! containers and chooses whether presenter notes are included.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use adocweave_core::OutputLimits;
use adocweave_core::output::diagnostics::{Diagnostic, DiagnosticCode, DiagnosticId, Severity};
use adocweave_core::output::html::{
    self, HtmlRegionError, HtmlRegionSelection, HtmlSlideScope, HtmlSlideSelections, RenderPolicy,
};
use adocweave_core::resolution::RenderInputs;
use adocweave_core::semantic::{
    self, Block, BlockId, BlockMetadata, DelimitedBlockKind, Document, HeadingKind, ListKind,
    SemanticNode,
};
use adocweave_core::text::TextRange;

pub(crate) mod bundle;
mod data;
pub(crate) mod helper;
mod styles;
mod svg;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
pub(crate) enum Audience {
    #[default]
    Public,
    Presenter,
}

#[derive(Clone, Debug)]
pub(crate) struct Slide {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) heading: Option<BlockId>,
    pub(crate) body: Vec<BlockId>,
    pub(crate) notes: Vec<BlockId>,
    pub(crate) columns: Option<[BlockId; 2]>,
    pub(crate) steps: BTreeMap<BlockId, u32>,
    pub(crate) hide_title: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct SlideGroup {
    /// A separate identity for the vertical stack, never the first slide's ID.
    pub(crate) id: String,
    pub(crate) slides: Vec<Slide>,
}

#[derive(Debug)]
pub(crate) struct Deck<'document> {
    pub(crate) document: &'document Document,
    pub(crate) groups: Vec<SlideGroup>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

#[derive(Debug)]
pub(crate) struct RenderedDeck {
    pub(crate) html: String,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

fn has_role(metadata: &BlockMetadata, role: &str) -> bool {
    metadata.role_names().any(|(name, _)| name == role)
}

fn has_option(metadata: &BlockMetadata, option: &str) -> bool {
    metadata.options.iter().any(|value| value.value == option)
        || metadata.attributes.iter().any(|attribute| {
            attribute.name.as_deref() == Some("options")
                && attribute
                    .value
                    .split(',')
                    .any(|value| value.trim() == option)
        })
}

fn unsupported_fragment_name(name: &str) -> bool {
    matches!(
        name,
        "fragment"
            | "step"
            | "fade-in"
            | "fade-out"
            | "fade-up"
            | "fade-down"
            | "fade-left"
            | "fade-right"
            | "grow"
            | "shrink"
            | "highlight-red"
            | "highlight-green"
            | "highlight-blue"
            | "fade-in-then-out"
            | "fade-in-then-semi-out"
            | "current-visible"
            | "semi-fade-out"
    )
}

fn problem(diagnostics: &mut Vec<Diagnostic>, code: &str, message: &str, range: TextRange) {
    diagnostics.push(Diagnostic {
        id: DiagnosticId::new(format!(
            "{code}@{}:{}",
            range.start().to_u32(),
            range.end().to_u32()
        )),
        code: DiagnosticCode::new(code),
        severity: Severity::Error,
        message: message.to_owned(),
        range,
        related: Vec::new(),
        fixes: Vec::new(),
    });
}

fn fresh_id(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut id = base.to_owned();
    let mut suffix = 2;
    while !used.insert(id.clone()) {
        id = format!("{base}_{suffix}");
        suffix += 1;
    }
    id
}

fn contains(outer: TextRange, inner: TextRange) -> bool {
    outer.start() <= inner.start() && inner.end() <= outer.end()
}

/// Retain references only to blocks and child lists; inline trees stay in core.
fn content_by_root(document: &Document) -> BTreeMap<BlockId, Vec<SemanticNode<'_>>> {
    let mut roots = BTreeMap::<_, Vec<_>>::new();
    let mut current = None;
    semantic::walk(document, |node| match node {
        SemanticNode::Block(block) => {
            let id = document
                .index()
                .block_id_at(block.range())
                .expect("block is indexed");
            if document.index().top_level_ordinal(id).is_some() {
                current = Some(id);
            }
            roots
                .entry(current.expect("block has a top-level parent"))
                .or_default()
                .push(node);
        }
        SemanticNode::List(list) if document.index().block_id_at(list.range).is_none() => {
            roots
                .entry(current.expect("list has a top-level parent"))
                .or_default()
                .push(node);
        }
        _ => {}
    });
    roots
}

impl<'document> Deck<'document> {
    pub(crate) fn contains_body_range(&self, range: TextRange) -> bool {
        !self.contains_note_range(range)
            && self
                .groups
                .iter()
                .flat_map(|group| &group.slides)
                .any(|slide| {
                    slide
                        .heading
                        .filter(|_| !slide.hide_title)
                        .into_iter()
                        .chain(slide.body.iter().copied())
                        .any(|id| self.block_owns_range(id, range))
                })
    }

    pub(crate) fn contains_note_range(&self, range: TextRange) -> bool {
        self.groups
            .iter()
            .flat_map(|group| &group.slides)
            .flat_map(|slide| &slide.notes)
            .any(|id| self.block_owns_range(*id, range))
    }
    fn block_owns_range(&self, id: BlockId, range: TextRange) -> bool {
        let block = self.document.block(id).expect("selected block exists");
        contains(block.range(), range)
            || block
                .metadata()
                .range
                .is_some_and(|metadata| contains(metadata, range))
    }

    pub(crate) fn compile(document: &'document Document) -> Self {
        let mut deck = Self {
            document,
            groups: Vec::new(),
            diagnostics: Vec::new(),
        };
        let heading_ids = document
            .heading_ids()
            .into_iter()
            .map(|heading| (heading.range, heading.id))
            .collect::<BTreeMap<_, _>>();
        let mut used = document
            .reference_targets()
            .iter()
            .map(|target| target.id.clone())
            .collect::<BTreeSet<_>>();
        let mut horizontal: Option<usize> = None;
        for (&id, block) in document
            .index()
            .top_level_blocks()
            .iter()
            .zip(document.blocks())
        {
            let boundary = match block {
                Block::Heading(heading) if heading.well_formed => match heading.kind {
                    HeadingKind::DocumentTitle => Some((heading, false)),
                    HeadingKind::Section { level: 1 } => Some((heading, false)),
                    HeadingKind::Section { level: 2 } => Some((heading, true)),
                    _ => None,
                },
                _ => None,
            };
            if let Some((heading, vertical)) =
                boundary.filter(|_| !has_role(block.metadata(), "notes"))
            {
                let slide = Slide {
                    id: heading_ids
                        .get(&heading.text_range)
                        .expect("semantic headings have IDs")
                        .clone(),
                    title: heading.text.clone(),
                    heading: Some(id),
                    body: Vec::new(),
                    notes: Vec::new(),
                    columns: None,
                    steps: BTreeMap::new(),
                    hide_title: has_option(&heading.metadata, "notitle"),
                };
                if vertical && horizontal.is_none() {
                    problem(
                        &mut deck.diagnostics,
                        "slides-missing-horizontal-parent",
                        "a vertical slide requires a preceding level-one slide",
                        heading.range,
                    );
                }
                if vertical && let Some(group) = horizontal {
                    deck.groups[group].slides.push(slide);
                } else {
                    let group = deck.groups.len();
                    let group_id = fresh_id(&format!("{}_stack", slide.id), &mut used);
                    deck.groups.push(SlideGroup {
                        id: group_id,
                        slides: vec![slide],
                    });
                    horizontal =
                        matches!(heading.kind, HeadingKind::Section { level: 1 }).then_some(group);
                }
                continue;
            }
            if deck.groups.is_empty() {
                let id = fresh_id("_preamble", &mut used);
                let group_id = fresh_id(&format!("{id}_stack"), &mut used);
                deck.groups.push(SlideGroup {
                    id: group_id,
                    slides: vec![Slide {
                        id,
                        title: String::new(),
                        heading: None,
                        body: Vec::new(),
                        notes: Vec::new(),
                        columns: None,
                        steps: BTreeMap::new(),
                        hide_title: false,
                    }],
                });
            }
            let current = deck
                .groups
                .last_mut()
                .expect("a current group exists")
                .slides
                .last_mut()
                .expect("a current slide exists");
            if has_role(block.metadata(), "notes") {
                current.notes.push(id);
                if !matches!(block, Block::Delimited(block) if block.kind == DelimitedBlockKind::Open && block.presentation.is_none())
                {
                    problem(
                        &mut deck.diagnostics,
                        "slides-invalid-notes",
                        "speaker notes require a plain open block",
                        block.range(),
                    );
                }
            } else {
                current.body.push(id);
            }
        }
        let content = content_by_root(document);
        deck.classify_nested_content(&content);
        deck.validate_layout(&content);
        semantic::walk(document, |node| {
            let SemanticNode::Metadata(metadata) = node else {
                return;
            };
            for (name, range) in metadata
                .role_names()
                .filter(|(name, _)| matches!(*name, "speaker" | "aside"))
            {
                problem(
                    &mut deck.diagnostics,
                    "slides-invalid-notes",
                    &format!(
                        "speaker note role `{name}` is not supported; use the notes role on a plain open block"
                    ),
                    range,
                );
            }
            for (name, range) in metadata
                .role_names()
                .filter(|(name, _)| unsupported_fragment_name(name))
            {
                let _ = name;
                problem(
                    &mut deck.diagnostics,
                    "slides-unsupported-fragment",
                    "fragment roles and effects are not supported; use the step option on a plain list or open block",
                    range,
                );
            }
            for option in metadata
                .options
                .iter()
                .filter(|option| option.value != "step" && unsupported_fragment_name(&option.value))
            {
                problem(
                    &mut deck.diagnostics,
                    "slides-unsupported-fragment",
                    "fragment effects are not supported",
                    option.range,
                );
            }
            for attribute in metadata.attributes.iter().filter(|attribute| {
                attribute.name.as_deref().is_some_and(|name| {
                    matches!(
                        name,
                        "fragment-index"
                            | "data-fragment-index"
                            | "fragment"
                            | "step"
                            | "effect"
                            | "widths"
                    ) || name.starts_with("revealjs_")
                })
            }) {
                problem(
                    &mut deck.diagnostics,
                    "slides-unsupported-fragment",
                    "explicit fragment ordering, effects and column widths are not supported",
                    attribute.range,
                );
            }
        });
        for attribute in document.attribute_occurrences() {
            if attribute.name.starts_with("revealjs_") || attribute.name == "notitle" {
                problem(
                    &mut deck.diagnostics,
                    "slides-unsupported-attribute",
                    "this presentation attribute is not supported",
                    attribute.range,
                );
            }
        }
        adocweave_core::output::diagnostics::sort_diagnostics(&mut deck.diagnostics);
        deck
    }

    fn classify_nested_content(
        &mut self,
        content: &BTreeMap<BlockId, Vec<SemanticNode<'document>>>,
    ) {
        for group in &mut self.groups {
            for slide in &mut group.slides {
                let mut roots = slide
                    .body
                    .iter()
                    .chain(&slide.notes)
                    .copied()
                    .collect::<Vec<_>>();
                roots.sort_by_key(|id| {
                    self.document
                        .index()
                        .block_range(*id)
                        .expect("root exists")
                        .start()
                });
                let nodes = roots
                    .iter()
                    .flat_map(|id| &content[id])
                    .copied()
                    .collect::<Vec<_>>();
                for node in &nodes {
                    let SemanticNode::Block(block) = node else {
                        continue;
                    };
                    if !has_role(block.metadata(), "notes") {
                        continue;
                    }
                    let id = self
                        .document
                        .index()
                        .block_id_at(block.range())
                        .expect("semantic block is indexed");
                    if slide.notes.contains(&id) {
                        continue;
                    }
                    if slide.notes.iter().any(|note| {
                        let range = self
                            .document
                            .index()
                            .block_range(*note)
                            .expect("note exists");
                        contains(range, block.range())
                    }) {
                        problem(
                            &mut self.diagnostics,
                            "slides-nested-notes",
                            "speaker notes cannot be nested",
                            block.range(),
                        );
                        continue;
                    }
                    if !matches!(block, Block::Delimited(block) if block.kind == DelimitedBlockKind::Open && block.presentation.is_none())
                    {
                        problem(
                            &mut self.diagnostics,
                            "slides-invalid-notes",
                            "speaker notes require a plain open block",
                            block.range(),
                        );
                    }
                    slide.notes.push(id);
                }
                slide.notes.sort_by_key(|id| {
                    self.document
                        .index()
                        .block_range(*id)
                        .expect("note exists")
                        .start()
                });
                let mut first = 0u32;
                let mut accepted = Vec::<TextRange>::new();
                for node in nodes {
                    let (metadata, range) = match node {
                        SemanticNode::Block(block) => (block.metadata(), block.range()),
                        SemanticNode::List(list) => (&list.metadata, list.range),
                        _ => unreachable!("content contains only blocks and child lists"),
                    };
                    if !has_option(metadata, "step") {
                        continue;
                    }
                    if slide.notes.iter().any(|id| {
                        contains(
                            self.document.index().block_range(*id).expect("note exists"),
                            range,
                        )
                    }) {
                        problem(
                            &mut self.diagnostics,
                            "slides-invalid-step",
                            "step presentation is not supported inside speaker notes",
                            range,
                        );
                        continue;
                    }
                    let SemanticNode::Block(block) = node else {
                        problem(
                            &mut self.diagnostics,
                            "slides-invalid-step",
                            "step presentation is not supported on a child list",
                            range,
                        );
                        continue;
                    };
                    if accepted.iter().any(|range| contains(*range, block.range())) {
                        problem(
                            &mut self.diagnostics,
                            "slides-nested-step",
                            "step blocks cannot be nested inside another step block",
                            block.range(),
                        );
                        continue;
                    }
                    let count = match block {
                        Block::List(list)
                            if matches!(list.kind, ListKind::Unordered | ListKind::Ordered) =>
                        {
                            list.items.len()
                        }
                        Block::Delimited(block)
                            if block.kind == DelimitedBlockKind::Open
                                && block.presentation.is_none() =>
                        {
                            1
                        }
                        _ => {
                            problem(
                                &mut self.diagnostics,
                                "slides-invalid-step",
                                "step presentation requires a plain unordered or ordered list or an open block",
                                block.range(),
                            );
                            continue;
                        }
                    };
                    let id = self
                        .document
                        .index()
                        .block_id_at(block.range())
                        .expect("step block exists");
                    slide.steps.insert(id, first);
                    first += u32::try_from(count).expect("analysis bounds list items");
                    accepted.push(block.range());
                }
            }
        }
    }

    fn validate_layout(&mut self, content: &BTreeMap<BlockId, Vec<SemanticNode<'document>>>) {
        for group in &mut self.groups {
            for slide in &mut group.slides {
                if let Some(id) = slide.heading {
                    let Block::Heading(heading) =
                        self.document.block(id).expect("slide heading exists")
                    else {
                        unreachable!()
                    };
                    if heading.text == "!" || has_option(&heading.metadata, "conceal") {
                        problem(
                            &mut self.diagnostics,
                            "slides-unsupported-title",
                            "use the notitle option to hide a slide heading",
                            heading.range,
                        );
                    }
                    if has_option(&heading.metadata, "step") {
                        problem(
                            &mut self.diagnostics,
                            "slides-invalid-step",
                            "step presentation is not supported on a slide heading",
                            heading.range,
                        );
                    }
                    if has_role(&heading.metadata, "column") {
                        problem(
                            &mut self.diagnostics,
                            "slides-misplaced-column",
                            "a column role requires a plain open block in a columns slide",
                            heading.range,
                        );
                    }
                    if has_role(&heading.metadata, "columns") {
                        let columns = slide
                            .body
                            .iter()
                            .copied()
                            .filter(|id| {
                                self.document
                                    .block(*id)
                                    .is_some_and(|block| has_role(block.metadata(), "column"))
                            })
                            .collect::<Vec<_>>();
                        let valid = columns.len() == 2 && slide.body.len() == 2 && columns.iter().all(|id| matches!(self.document.block(*id), Some(Block::Delimited(block)) if block.kind == DelimitedBlockKind::Open && block.presentation.is_none()));
                        if valid {
                            slide.columns = Some([columns[0], columns[1]]);
                        } else {
                            problem(
                                &mut self.diagnostics,
                                "slides-invalid-columns",
                                "a columns slide requires exactly two plain column open blocks and no other body blocks",
                                heading.range,
                            );
                        }
                    }
                }
                for node in slide.body.iter().flat_map(|id| &content[id]) {
                    let (metadata, range, id) = match node {
                        SemanticNode::Block(block) => (
                            block.metadata(),
                            block.range(),
                            self.document.index().block_id_at(block.range()),
                        ),
                        SemanticNode::List(list) => (&list.metadata, list.range, None),
                        _ => unreachable!("content contains only blocks and child lists"),
                    };
                    if slide.notes.iter().any(|id| {
                        contains(
                            self.document.index().block_range(*id).expect("note exists"),
                            range,
                        )
                    }) {
                        continue;
                    }
                    if has_role(metadata, "column")
                        && !id.is_some_and(|id| {
                            slide.columns.is_some_and(|columns| columns.contains(&id))
                        })
                    {
                        problem(
                            &mut self.diagnostics,
                            "slides-misplaced-column",
                            "a column block requires a valid columns slide",
                            range,
                        );
                    }
                    if has_role(metadata, "columns") {
                        problem(
                            &mut self.diagnostics,
                            "slides-misplaced-columns",
                            "columns is only supported on slide headings",
                            range,
                        );
                    }
                    if has_option(metadata, "notitle") {
                        problem(
                            &mut self.diagnostics,
                            "slides-misplaced-notitle",
                            "notitle is only supported on slide headings",
                            range,
                        );
                    }
                }
            }
        }
    }

    fn body_regions(&self) -> Vec<Vec<HtmlRegionSelection>> {
        self.groups
            .iter()
            .flat_map(|group| &group.slides)
            .map(|slide| {
                let mut heading = slide
                    .heading
                    .filter(|_| !slide.hide_title)
                    .into_iter()
                    .collect::<Vec<_>>();
                let mut selections = Vec::new();
                if let Some(columns) = slide.columns {
                    selections.push(heading);
                    selections.extend(columns.into_iter().map(|id| vec![id]));
                } else {
                    heading.extend(&slide.body);
                    selections.push(heading);
                }
                selections
                    .into_iter()
                    .enumerate()
                    .map(|(region, blocks)| HtmlRegionSelection {
                        omitted_blocks: slide.notes.iter().copied().collect(),
                        stepped_blocks: slide
                            .steps
                            .iter()
                            .filter(|(id, _)| {
                                let target = self
                                    .document
                                    .index()
                                    .block_range(**id)
                                    .expect("step exists");
                                blocks.iter().any(|id| {
                                    let range = self
                                        .document
                                        .index()
                                        .block_range(*id)
                                        .expect("region block exists");
                                    range.start() <= target.start() && target.end() <= range.end()
                                })
                            })
                            .map(|(&id, &first)| (id, first))
                            .collect(),
                        container_headings: (region == 0)
                            .then_some(slide.heading)
                            .flatten()
                            .into_iter()
                            .collect(),
                        blocks,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn note_regions(&self) -> Vec<Vec<HtmlRegionSelection>> {
        self.groups
            .iter()
            .flat_map(|group| &group.slides)
            .map(|slide| {
                vec![HtmlRegionSelection {
                    blocks: slide.notes.clone(),
                    ..Default::default()
                }]
            })
            .collect()
    }

    pub(crate) fn reserved_ids(&self) -> BTreeSet<String> {
        self.document
            .reference_targets()
            .iter()
            .map(|target| target.id.clone())
            .chain(self.groups.iter().map(|group| group.id.clone()))
            .chain(
                self.groups
                    .iter()
                    .flat_map(|group| group.slides.iter().map(|slide| slide.id.clone())),
            )
            .collect()
    }

    pub(crate) fn render(
        &self,
        audience: Audience,
        policy: &RenderPolicy,
        body_inputs: &RenderInputs,
        note_inputs: &RenderInputs,
        limits: OutputLimits,
    ) -> Result<RenderedDeck, HtmlRegionError> {
        let selections = HtmlSlideSelections {
            body: self.body_regions(),
            notes: if audience == Audience::Presenter {
                self.note_regions()
            } else {
                Vec::new()
            },
        };
        let mut reserved = self.reserved_ids();
        let body = html::render_slide_regions(
            self.document,
            policy,
            body_inputs,
            &selections,
            HtmlSlideScope::Body,
            &reserved,
            limits,
        )?;
        reserved.extend(body.generated_ids.iter().cloned());
        let notes = if audience == Audience::Presenter {
            html::render_slide_regions(
                self.document,
                policy,
                note_inputs,
                &selections,
                HtmlSlideScope::Notes,
                &reserved,
                limits,
            )?
        } else {
            Default::default()
        };
        let mut output = String::from("<div class=\"reveal\">\n<div class=\"slides\">\n");
        let mut body_regions = body.regions.iter();
        let mut note_regions = notes.regions.iter();
        let mut body_footnotes = body.footnotes.iter();
        let mut note_footnotes = notes.footnotes.iter();
        let slide_count = self
            .groups
            .iter()
            .map(|group| group.slides.len())
            .sum::<usize>();
        let mut slide_index = 0;
        for group in &self.groups {
            if group.slides.len() > 1 {
                writeln!(
                    output,
                    "<section id=\"{}\" role=\"group\" aria-label=\"{}\">",
                    group.id,
                    bundle::escape(&group.slides[0].title)
                )
                .expect("writing to a String cannot fail");
            }
            for slide in &group.slides {
                debug_assert!(semantic::is_valid_anchor_id(&slide.id));
                writeln!(output, "<section id=\"{}\">", slide.id)
                    .expect("writing to a String cannot fail");
                output.push_str(
                    body_regions
                        .next()
                        .expect("every slide has one body region"),
                );
                if slide.columns.is_some() {
                    output.push_str("<div class=\"columns\">\n");
                    for _ in 0..2 {
                        output.push_str("<div class=\"column\">\n");
                        output.push_str(
                            body_regions
                                .next()
                                .expect("every column has one body region"),
                        );
                        output.push_str("</div>\n");
                    }
                    output.push_str("</div>\n");
                }
                output.push_str(
                    body_footnotes
                        .next()
                        .expect("one footnote region per slide"),
                );
                let note = note_regions.next().map(String::as_str).unwrap_or("");
                let footnotes = note_footnotes.next().map(String::as_str).unwrap_or("");
                let bibliography = (slide_index + 1 == slide_count)
                    .then_some(notes.bibliography.as_deref())
                    .flatten()
                    .unwrap_or("");
                if !note.is_empty() || !footnotes.is_empty() || !bibliography.is_empty() {
                    output.push_str("<aside class=\"notes\">\n");
                    // The stock notes plugin copies this finite content into
                    // its independent popup, which does not load our theme.
                    output.push_str("<link rel=\"stylesheet\" href=\"assets/content.css\">\n");
                    output.push_str(note);
                    output.push_str(footnotes);
                    output.push_str(bibliography);
                    output.push_str("</aside>\n");
                }
                output.push_str("</section>\n");
                slide_index += 1;
            }
            if group.slides.len() > 1 {
                output.push_str("</section>\n");
            }
        }
        if let Some(bibliography) = &body.bibliography {
            output.push_str("<section id=\"slides-body-references\">\n<h2>References</h2>\n");
            output.push_str(bibliography);
            output.push_str("</section>\n");
        }
        output.push_str("</div>\n</div>\n");
        if output.len() > limits.max_output_bytes as usize {
            return Err(HtmlRegionError::OutputLimit {
                limit: limits.max_output_bytes,
                actual: output.len(),
            });
        }
        let mut diagnostics = self.diagnostics.clone();
        diagnostics.extend(body.diagnostics);
        diagnostics.extend(notes.diagnostics);
        adocweave_core::output::diagnostics::sort_diagnostics(&mut diagnostics);
        Ok(RenderedDeck {
            html: output,
            diagnostics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adocweave_core::{AnalysisOptions, Engine};

    fn render(deck: &Deck<'_>, audience: Audience) -> RenderedDeck {
        deck.render(
            audience,
            &RenderPolicy::default(),
            &RenderInputs::default(),
            &RenderInputs::default(),
            OutputLimits::default(),
        )
        .unwrap()
    }

    #[test]
    fn public_and_presenter_share_slide_ids_and_steps_without_leaking_nested_notes() {
        let analysis = Engine::new(AnalysisOptions::default()).analyze("= Talk\n\n[#method]\n== Method\n\n[%step]\n* <script>alert(1)</script>\n* Result\n\n====\nPublic text.\n\n[.notes]\n--\nPRIVATE_NOTE\nimage::private.png[Secret]\n--\n====\n\n== Evaluation\n\nVisible.\n").unwrap();
        let deck = Deck::compile(analysis.document());
        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        assert_eq!(deck.groups.len(), 3);
        assert_eq!(deck.groups[1].slides[0].title, "Method");
        assert_eq!(deck.groups[1].slides[0].id, "method");
        let public = render(&deck, Audience::Public);
        let presenter = render(&deck, Audience::Presenter);
        assert!(public.diagnostics.is_empty());
        assert_eq!(presenter.diagnostics.len(), 1);
        assert_eq!(
            presenter.diagnostics[0].code.as_str(),
            "unresolved-resource"
        );
        assert!(!public.html.contains("PRIVATE_NOTE"));
        assert!(!public.html.contains("private.png"));
        assert!(presenter.html.contains("<aside class=\"notes\">"));
        assert!(presenter.html.contains(
            "<aside class=\"notes\">\n<link rel=\"stylesheet\" href=\"assets/content.css\">"
        ));
        assert!(presenter.html.contains("PRIVATE_NOTE"));
        assert!(
            public
                .html
                .contains("&lt;script&gt;alert(1)&lt;/script&gt;")
        );
        assert!(!public.html.contains("<script>"));
        assert_eq!(public.html.matches("id=\"method\"").count(), 1);
        for html in [&public.html, &presenter.html] {
            assert!(html.contains("<li class=\"fragment\" data-fragment-index=\"0\">"));
            assert!(html.contains("<li class=\"fragment\" data-fragment-index=\"1\">Result"));
        }
        let ordinary = html::render(analysis.document(), &RenderPolicy::default());
        assert!(ordinary.html.contains("PRIVATE_NOTE"));
        assert!(!ordinary.html.contains("class=\"fragment\""));
        let ordinary = html::render(
            analysis.document(),
            &RenderPolicy {
                roles: html::RolePolicy {
                    allowed: BTreeSet::from(["notes".to_owned()]),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        assert!(ordinary.html.contains("class=\"open role-notes\""));
        assert!(ordinary.html.contains("PRIVATE_NOTE"));
    }

    #[test]
    fn open_steps_number_whole_blocks_and_all_output_shares_one_limit() {
        let analysis = Engine::new(AnalysisOptions::default()).analyze("= Talk\n\n== Method\n\n[%step]\n--\nWhole block.\n--\n\n[%step]\n. First\n. Second\n\n[.notes]\n--\nPrivate detail.\n--\n").unwrap();
        let deck = Deck::compile(analysis.document());
        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        let public = render(&deck, Audience::Public);
        let presenter = render(&deck, Audience::Presenter);
        assert!(
            public.html.contains(
                "<div class=\"fragment\" data-fragment-index=\"0\">\n<div class=\"open\">"
            )
        );
        assert!(
            public
                .html
                .contains("<li class=\"fragment\" data-fragment-index=\"1\">First")
        );
        assert!(
            public
                .html
                .contains("<li class=\"fragment\" data-fragment-index=\"2\">Second")
        );
        let limits = OutputLimits {
            max_output_bytes: public.html.len() as u32,
        };
        assert!(
            deck.render(
                Audience::Public,
                &RenderPolicy::default(),
                &RenderInputs::default(),
                &RenderInputs::default(),
                limits
            )
            .is_ok()
        );
        assert_eq!(
            deck.render(
                Audience::Presenter,
                &RenderPolicy::default(),
                &RenderInputs::default(),
                &RenderInputs::default(),
                limits
            )
            .unwrap_err(),
            HtmlRegionError::OutputLimit {
                limit: limits.max_output_bytes,
                actual: presenter.html.len()
            }
        );
    }

    #[test]
    fn misplaced_markers_and_nested_notes_report_their_original_ranges() {
        let source = "= Talk\n\n[%step]\n== Method\n\n[.columns]\n--\n[%notitle]\nA paragraph.\n\n[.column]\n====\nInner.\n====\n--\n\n[.notes]\n--\nPrivate outer.\n\n[.notes]\n====\nPrivate inner.\n====\n--\n\n[.notes]\n== Private heading\n";
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze(source)
            .unwrap();
        let deck = Deck::compile(analysis.document());
        assert_eq!(
            deck.groups.len(),
            2,
            "a notes heading must not create another slide"
        );
        for code in [
            "slides-invalid-step",
            "slides-misplaced-columns",
            "slides-misplaced-column",
            "slides-misplaced-notitle",
            "slides-nested-notes",
            "slides-invalid-notes",
        ] {
            assert!(
                deck.diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code.as_str() == code),
                "{code}: {:?}",
                deck.diagnostics
            );
        }
        let public = render(&deck, Audience::Public);
        assert!(!public.html.contains("Private"));
        let presenter = render(&deck, Audience::Presenter);
        assert_eq!(presenter.html.matches("Private inner.").count(), 1);
        assert_eq!(presenter.html.matches("Private outer.").count(), 1);
        let nested = deck
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code.as_str() == "slides-nested-notes")
            .unwrap();
        assert!(
            source[nested.range.start().to_u32() as usize..nested.range.end().to_u32() as usize]
                .contains("Private inner.")
        );
    }

    #[test]
    fn vertical_slides_columns_notitle_and_deep_headings_preserve_targets() {
        let analysis = Engine::new(AnalysisOptions::default()).analyze("= Talk\n\n[#method.columns]\n== Method\n\n[.column]\n--\nLeft.\n--\n\n[.column]\n--\nRight.\n--\n\n[#result%notitle]\n=== Result\n\n[#conditions]\n==== Conditions\n\nDetails.\n\n== Conclusion\n").unwrap();
        let deck = Deck::compile(analysis.document());
        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        assert_eq!(deck.groups[1].slides.len(), 2);
        assert!(deck.groups[1].slides[0].columns.is_some());
        assert!(deck.groups[1].slides[1].hide_title);
        let output = render(&deck, Audience::Public);
        assert!(
            output
                .html
                .contains("<section id=\"method_stack\" role=\"group\" aria-label=\"Method\">\n<section id=\"method\">")
        );
        assert!(
            output
                .html
                .contains("<div class=\"columns\">\n<div class=\"column\">")
        );
        assert!(
            output
                .html
                .contains("<section id=\"result\">\n<h3 id=\"conditions\">Conditions</h3>")
        );
        assert!(!output.html.contains(">Result</h2>"));
        assert_eq!(output.html.matches("id=\"conditions\"").count(), 1);
    }

    #[test]
    fn slide_headings_follow_one_document_title_and_vertical_group_names_are_escaped() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze("= Talk\n\n[#method]\n== Method \"quoted\" & <unsafe>\n\n[#child]\n=== Child\n\n==== Detail\n")
            .unwrap();
        let deck = Deck::compile(analysis.document());
        assert!(deck.diagnostics.is_empty(), "{:?}", deck.diagnostics);
        let output = render(&deck, Audience::Public);
        assert_eq!(output.html.matches("<h1").count(), 1);
        assert_eq!(output.html.matches("<h2").count(), 2);
        assert!(output.html.contains("<h3 id=\"_detail\">Detail</h3>"));
        assert!(output.html.contains(
            "role=\"group\" aria-label=\"Method &quot;quoted&quot; &amp; &lt;unsafe&gt;\""
        ));
        let ordinary = html::render(analysis.document(), &RenderPolicy::default());
        assert!(ordinary.html.contains("<h1 id=\"method\">"));
        assert!(ordinary.html.contains("<h2 id=\"child\">"));
    }

    #[test]
    fn malformed_layout_has_source_located_diagnostics_and_private_content_stays_out() {
        let source = "= Talk\n\n=== Orphan\n\n[.columns]\n== Bad columns\n\n[.column]\n--\nOnly one.\n--\n\n[%step]\nParagraph.\n\n[.notes]\nSECRET\n";
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze(source)
            .unwrap();
        let deck = Deck::compile(analysis.document());
        for code in [
            "slides-missing-horizontal-parent",
            "slides-invalid-columns",
            "slides-invalid-step",
            "slides-invalid-notes",
        ] {
            assert!(
                deck.diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code.as_str() == code),
                "{code}: {:?}",
                deck.diagnostics
            );
        }
        for diagnostic in &deck.diagnostics {
            assert!(
                source
                    .get(
                        diagnostic.range.start().to_u32() as usize
                            ..diagnostic.range.end().to_u32() as usize
                    )
                    .is_some()
            );
        }
        assert!(!render(&deck, Audience::Public).html.contains("SECRET"));
    }

    #[test]
    fn include_diagnostic_ranges_map_to_original_source_and_block_ids_are_retained() {
        use adocweave_core::preprocess::{
            EffectiveProcessingOptions, ExpandedRange, PreprocessInputs, PreprocessOptions,
            ResourceDocument, ResourceSnapshot,
        };
        let mut resources = ResourceSnapshot::default();
        resources.insert(
            "part.adoc",
            ResourceDocument {
                source_id: adocweave_core::SourceId::new("part"),
                source: "[%step]\nNot a list.\n".into(),
            },
        );
        let prepared = EffectiveProcessingOptions::new(
            AnalysisOptions::default(),
            PreprocessOptions::default(),
        )
        .unwrap()
        .preprocess_and_analyze(
            "= Talk\n\n== Method\n\ninclude::part.adoc[]\n",
            &resources,
            PreprocessInputs::default(),
        )
        .unwrap();
        let deck = Deck::compile(prepared.analysis.document());
        let diagnostic = deck
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code.as_str() == "slides-invalid-step")
            .unwrap();
        let origins = prepared
            .document
            .origins_for_range(ExpandedRange::new(diagnostic.range));
        assert_eq!(origins.len(), 1);
        assert_eq!(origins[0].source_id.as_ref().unwrap().as_str(), "part");
        let id = deck.groups[1].slides[0].body[0];
        assert_eq!(
            deck.document.index().block_range(id),
            Some(diagnostic.range)
        );
    }
}
