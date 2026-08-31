//! Deterministic, host-independent projections derived from one [`Analysis`].

use std::collections::BTreeSet;

use crate::block_model::AstBlock;
use crate::core::{Analysis, SourceId};
use crate::inline_model::{Inline, Link};
use crate::reference::{ReferenceKey, ResolutionOutcome};
use crate::render::{RenderInputs, ResolutionMatch};
use crate::source::TextRange;

/// Semantic features that a host may need to render a document.
///
/// The values describe document content only. They do not select renderer
/// implementations, JavaScript libraries, themes, or asset URLs.
#[non_exhaustive]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RenderingFeatures {
    pub math_languages: Vec<String>,
    pub source_languages: Vec<String>,
    pub table_of_contents: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceBlockProjection {
    /// The delimited source block from the opening delimiter through the
    /// closing delimiter. Preceding title, anchor, and attribute lines are not
    /// part of this range.
    pub source_range: TextRange,
    pub content_range: TextRange,
    pub title: Option<ProjectedText>,
    pub language_range: Option<TextRange>,
    pub language: Option<String>,
    pub line_numbers: bool,
    pub start_line: Option<u32>,
    pub source: String,
    /// The numbered caption label (`Listing 1`) when the document sets
    /// `listing-caption` and the block has a title.
    pub caption: Option<String>,
}

/// Presentation facts for an ordered list, resolved once during lowering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrderedListProjection {
    pub source_range: TextRange,
    pub start: Option<u32>,
    pub reversed: bool,
    pub style: crate::block_model::OrderedListStyle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockPresentationKind {
    Admonition,
    Quote,
    Verse,
    Example,
    Sidebar,
    Open,
    Collapsible,
    Figure,
    Table,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockPresentationProjection {
    pub kind: BlockPresentationKind,
    pub source_range: TextRange,
    pub content_range: TextRange,
    pub title: Option<String>,
    pub attribution: Option<String>,
    pub citation: Option<String>,
    /// Roles written on the block, independent of any render policy.
    pub roles: Vec<String>,
    /// For a collapsible block, whether it starts expanded.
    pub open: Option<bool>,
    /// The numbered caption label (`Figure 1`) of a titled figure, table, or
    /// example, when the document numbers it.
    pub caption: Option<String>,
}

/// The presentation facts of a delimited block, if it has any a host can use:
/// the typed presentations, and the compound containers a reader can see.
fn delimited_presentation(
    block: &crate::block_model::DelimitedBlock,
    presentation: &crate::presentation::DocumentPresentation,
) -> Option<BlockPresentationProjection> {
    use crate::block_model::{DelimitedBlockKind, DelimitedContent, DelimitedPresentation};
    let title = block
        .metadata
        .title
        .as_ref()
        .map(|item| resolved_inline_text(&item.inlines));
    let (kind, attribution, citation, open) = match &block.presentation {
        Some(DelimitedPresentation::Admonition(_)) => {
            (BlockPresentationKind::Admonition, None, None, None)
        }
        Some(DelimitedPresentation::Quote(quote)) => (
            match quote.kind {
                crate::block_model::QuoteKind::Quote => BlockPresentationKind::Quote,
                crate::block_model::QuoteKind::Verse => BlockPresentationKind::Verse,
            },
            quote.attribution.as_ref().map(|item| item.value.clone()),
            quote.citation.as_ref().map(|item| item.value.clone()),
            None,
        ),
        Some(DelimitedPresentation::Collapsible(collapsible)) => (
            BlockPresentationKind::Collapsible,
            None,
            None,
            Some(collapsible.open),
        ),
        None => {
            let kind = match (&block.content, block.kind) {
                (DelimitedContent::Table(_), _) => BlockPresentationKind::Table,
                (DelimitedContent::Compound(_), DelimitedBlockKind::Example) => {
                    BlockPresentationKind::Example
                }
                (DelimitedContent::Compound(_), DelimitedBlockKind::Sidebar) => {
                    BlockPresentationKind::Sidebar
                }
                (DelimitedContent::Compound(_), DelimitedBlockKind::Open) => {
                    BlockPresentationKind::Open
                }
                _ => return None,
            };
            (kind, None, None, None)
        }
    };
    Some(BlockPresentationProjection {
        kind,
        source_range: block.range,
        content_range: block.content_range,
        title,
        attribution,
        citation,
        roles: block_role_names(&block.metadata),
        open,
        caption: presentation
            .caption_at(block.range)
            .and_then(crate::caption::BlockCaption::label),
    })
}

/// An image block is a figure in the structure information, titled or not.
fn image_block_presentation(
    paragraph: &crate::block_model::Paragraph,
    presentation: &crate::presentation::DocumentPresentation,
) -> Option<BlockPresentationProjection> {
    crate::caption::block_image(paragraph)?;
    Some(BlockPresentationProjection {
        kind: BlockPresentationKind::Figure,
        source_range: paragraph.range,
        content_range: paragraph.content_range,
        title: paragraph
            .metadata
            .title
            .as_ref()
            .map(|item| resolved_inline_text(&item.inlines)),
        attribution: None,
        citation: None,
        roles: block_role_names(&paragraph.metadata),
        open: None,
        caption: presentation
            .caption_at(paragraph.range)
            .and_then(crate::caption::BlockCaption::label),
    })
}

fn block_role_names(metadata: &crate::block_model::BlockMetadata) -> Vec<String> {
    metadata
        .role_names()
        .map(|(role, _)| role.to_owned())
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormulaKind {
    Inline,
    Block,
}

impl FormulaKind {
    /// Stable display-form name used by serialized and HTML contracts.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Block => "block",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormulaProjection {
    pub kind: FormulaKind,
    pub language: crate::inline_model::MathLanguage,
    pub source_range: TextRange,
    pub content_range: TextRange,
    pub source: String,
}

impl FormulaProjection {
    /// Inline or block display form without inferring it from source syntax.
    pub const fn display(&self) -> FormulaKind {
        self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectedText {
    pub source_range: TextRange,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalLink {
    pub source_range: TextRange,
    pub target_range: TextRange,
    pub target: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferenceEdge {
    pub source_id: Option<SourceId>,
    pub source_range: TextRange,
    pub target: ReferenceKey,
    pub resolution: Option<ResolutionOutcome>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchTextKind {
    Prose,
    Code,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchTextSegment {
    pub kind: SearchTextKind,
    pub source_range: TextRange,
    pub text: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchableText {
    pub text: String,
    pub segments: Vec<SearchTextSegment>,
}

/// Returns the document title's visible text and source range, if present.
pub fn document_title(analysis: &Analysis) -> Option<ProjectedText> {
    analysis
        .ast()
        .blocks()
        .iter()
        .find_map(|block| match block {
            AstBlock::Heading(heading)
                if matches!(heading.kind, crate::block_model::HeadingKind::DocumentTitle) =>
            {
                Some(ProjectedText {
                    source_range: heading.text_range,
                    text: inline_text(&heading.inlines),
                })
            }
            _ => None,
        })
}

/// Returns authored external links in source order.
///
/// An empty label falls back to the authored target text.
pub fn external_links(analysis: &Analysis) -> Vec<ExternalLink> {
    let mut external_links = analysis
        .links()
        .iter()
        .map(project_link)
        .collect::<Vec<_>>();
    external_links.sort_by_key(|link| (link.source_range.start(), link.source_range.end()));
    external_links
}

/// Returns typed reference edges with uniquely matching host resolutions.
///
/// A missing or duplicate resolution at the exact reference range leaves the
/// edge unresolved.
pub fn reference_edges(analysis: &Analysis, inputs: &RenderInputs) -> Vec<ReferenceEdge> {
    analysis
        .references()
        .iter()
        .filter_map(|reference| {
            let target = reference.target.clone()?;
            let resolution = match inputs.reference_at(reference.range) {
                ResolutionMatch::Unique(resolution) => Some(resolution.outcome.clone()),
                ResolutionMatch::Missing | ResolutionMatch::Duplicate => None,
            };
            Some(ReferenceEdge {
                source_id: analysis.source_id().cloned(),
                source_range: reference.range,
                target,
                resolution,
            })
        })
        .collect()
}

/// Returns source blocks and their resolved display facts in source order.
pub fn source_blocks(analysis: &Analysis) -> Vec<SourceBlockProjection> {
    let mut source_blocks = Vec::new();
    crate::walker::walk(analysis.document(), |node| {
        if let crate::walker::SemanticNode::Block(AstBlock::Verbatim(block)) = node
            && let crate::block_model::VerbatimKind::Source(source) = &block.kind
        {
            source_blocks.push(SourceBlockProjection {
                source_range: block.range,
                content_range: block.content_range,
                title: block.metadata.title.as_ref().map(|title| ProjectedText {
                    source_range: title.range,
                    text: resolved_inline_text(&title.inlines),
                }),
                language_range: source.language_range,
                language: source.language.clone(),
                line_numbers: source.line_numbers,
                start_line: source.start_line,
                source: block.value.clone(),
                caption: analysis
                    .presentation()
                    .caption_at(block.range)
                    .and_then(crate::caption::BlockCaption::label),
            });
        }
    });
    source_blocks.sort_by_key(|source| (source.source_range.start(), source.source_range.end()));
    source_blocks
}

/// Returns inline and block formulas in source order.
pub fn formulas(analysis: &Analysis) -> Vec<FormulaProjection> {
    let mut formulas = Vec::new();
    crate::walker::walk(analysis.document(), |node| match node {
        crate::walker::SemanticNode::Inline(Inline::Formula(formula)) => {
            formulas.push(FormulaProjection {
                kind: FormulaKind::Inline,
                language: formula.language,
                source_range: formula.range,
                content_range: formula.content_range,
                source: formula.value.clone(),
            });
        }
        crate::walker::SemanticNode::Block(AstBlock::Math(formula)) => {
            formulas.push(FormulaProjection {
                kind: FormulaKind::Block,
                language: formula.language,
                source_range: formula.range,
                content_range: formula.content_range,
                source: formula.value.clone(),
            });
        }
        _ => {}
    });
    formulas.sort_by_key(|formula| (formula.source_range.start(), formula.source_range.end()));
    formulas
}

/// Returns the resolved presentation of ordered lists in source order.
pub fn ordered_lists(analysis: &Analysis) -> Vec<OrderedListProjection> {
    let mut ordered_lists = Vec::new();
    crate::walker::walk(analysis.document(), |node| {
        if let crate::walker::SemanticNode::Block(AstBlock::List(list)) = node
            && list.kind == crate::block_model::ListKind::Ordered
        {
            ordered_lists.push(OrderedListProjection {
                source_range: list.range,
                start: list.presentation.start,
                reversed: list.presentation.reversed,
                style: list.presentation.style,
            });
        }
    });
    ordered_lists.sort_by_key(|list| (list.source_range.start(), list.source_range.end()));
    ordered_lists
}

/// Returns visible block presentation facts in source order.
pub fn block_presentations(analysis: &Analysis) -> Vec<BlockPresentationProjection> {
    let mut block_presentations = Vec::new();
    crate::walker::walk(analysis.document(), |node| match node {
        crate::walker::SemanticNode::Block(AstBlock::Paragraph(value))
            if value.admonition.is_some() =>
        {
            block_presentations.push(BlockPresentationProjection {
                kind: BlockPresentationKind::Admonition,
                source_range: value.range,
                content_range: value.content_range,
                title: value
                    .metadata
                    .title
                    .as_ref()
                    .map(|value| value.value.clone()),
                attribution: None,
                citation: None,
                roles: block_role_names(&value.metadata),
                open: None,
                caption: None,
            });
        }
        crate::walker::SemanticNode::Block(AstBlock::Paragraph(value)) => {
            if let Some(figure) = image_block_presentation(value, analysis.presentation()) {
                block_presentations.push(figure);
            }
        }
        crate::walker::SemanticNode::Block(AstBlock::Delimited(value)) => {
            if let Some(presentation) = delimited_presentation(value, analysis.presentation()) {
                block_presentations.push(presentation);
            }
        }
        _ => {}
    });
    block_presentations.sort_by_key(|block| (block.source_range.start(), block.source_range.end()));
    block_presentations
}

/// Returns searchable prose and code with ranges into the analyzed source.
pub fn searchable_text(analysis: &Analysis) -> SearchableText {
    let mut segments = Vec::new();
    collect_search_blocks(analysis.ast().blocks(), &mut segments);
    let text = segments
        .iter()
        .map(|segment| segment.text.as_str())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    SearchableText { text, segments }
}

fn project_link(link: &Link) -> ExternalLink {
    let label = inline_text(&link.label);
    ExternalLink {
        source_range: link.range,
        target_range: link.target_range,
        target: link.target.clone(),
        label: if label.is_empty() {
            link.target.clone()
        } else {
            label
        },
    }
}

const fn role_search_kind(role: crate::text_role::BlockTextRole) -> Option<SearchTextKind> {
    match role {
        crate::text_role::BlockTextRole::Prose => Some(SearchTextKind::Prose),
        crate::text_role::BlockTextRole::Code => Some(SearchTextKind::Code),
        crate::text_role::BlockTextRole::Container | crate::text_role::BlockTextRole::Excluded => {
            None
        }
    }
}

fn push_search_role(
    output: &mut Vec<SearchTextSegment>,
    role: crate::text_role::BlockTextRole,
    source_range: TextRange,
    text: String,
) {
    if let Some(kind) = role_search_kind(role) {
        push_search(output, kind, source_range, text);
    }
}

fn collect_search_blocks(blocks: &[AstBlock], output: &mut Vec<SearchTextSegment>) {
    crate::walker::walk_block_slice(blocks, |node| match node {
        crate::walker::SemanticNode::Block(block @ AstBlock::Heading(heading)) => push_search_role(
            output,
            crate::text_role::block_text_role(block),
            heading.text_range,
            inline_text(&heading.inlines),
        ),
        crate::walker::SemanticNode::Block(block @ AstBlock::Paragraph(paragraph)) => {
            push_search_role(
                output,
                crate::text_role::block_text_role(block),
                paragraph.content_range,
                fold_line_endings(&inline_text(&paragraph.inlines)),
            );
        }
        crate::walker::SemanticNode::Block(block @ AstBlock::LiteralParagraph(literal)) => {
            push_search_role(
                output,
                crate::text_role::block_text_role(block),
                literal.content_range,
                literal.value.clone(),
            );
        }
        crate::walker::SemanticNode::Block(block @ AstBlock::Verbatim(_)) => {
            let (content_range, value) = match block {
                AstBlock::Verbatim(source) => (source.content_range, source.value.clone()),
                _ => unreachable!("the pattern admits only source and verbatim blocks"),
            };
            push_search_role(
                output,
                crate::text_role::block_text_role(block),
                content_range,
                value,
            );
        }
        crate::walker::SemanticNode::Block(AstBlock::Delimited(block)) => {
            if let crate::block_model::DelimitedContent::Verbatim(value) = &block.content
                && matches!(
                    crate::text_role::delimited_text_role(block.kind),
                    crate::text_role::BlockTextRole::Code
                )
            {
                push_search(
                    output,
                    SearchTextKind::Code,
                    block.content_range,
                    value.clone(),
                );
            }
        }
        crate::walker::SemanticNode::ListItem(item) => {
            for term in &item.terms {
                push_search(
                    output,
                    SearchTextKind::Prose,
                    term.range,
                    inline_text(&term.inlines),
                );
            }
            push_search(
                output,
                SearchTextKind::Prose,
                item.text_range,
                inline_text(&item.inlines),
            );
        }
        crate::walker::SemanticNode::TableCell(cell) => {
            let role = crate::text_role::table_cell_text_role(&cell.content);
            match &cell.content {
                crate::table::TableCellContent::Inlines(inlines) => {
                    push_search_role(output, role, cell.content_range, inline_text(inlines));
                }
                crate::table::TableCellContent::Verbatim(value) => {
                    push_search_role(output, role, cell.content_range, value.clone());
                }
                crate::table::TableCellContent::AsciiDoc(_) => {}
            }
        }
        crate::walker::SemanticNode::Block(
            AstBlock::Break(_) | AstBlock::List(_) | AstBlock::Math(_) | AstBlock::Unsupported(_),
        )
        | crate::walker::SemanticNode::List(_)
        | crate::walker::SemanticNode::Table(_)
        | crate::walker::SemanticNode::TableRow(_)
        | crate::walker::SemanticNode::Inline(_)
        | crate::walker::SemanticNode::Attribute(_)
        | crate::walker::SemanticNode::Anchor(_)
        | crate::walker::SemanticNode::Metadata(_)
        | crate::walker::SemanticNode::MetadataTitle(_)
        | crate::walker::SemanticNode::MetadataId(_)
        | crate::walker::SemanticNode::MetadataRole(_)
        | crate::walker::SemanticNode::MetadataOption(_)
        | crate::walker::SemanticNode::ElementAttribute(_) => {}
    });
}

fn push_search(
    output: &mut Vec<SearchTextSegment>,
    kind: SearchTextKind,
    source_range: TextRange,
    text: String,
) {
    let text = text.trim_end_matches(['\r', '\n']).to_owned();
    if !text.is_empty() {
        output.push(SearchTextSegment {
            kind,
            source_range,
            text,
        });
    }
}

fn inline_text(inlines: &[Inline]) -> String {
    inline_text_with_attributes(inlines, false)
}

pub(crate) fn resolved_inline_text(inlines: &[Inline]) -> String {
    inline_text_with_attributes(inlines, true)
}

fn inline_text_with_attributes(inlines: &[Inline], include_attribute_values: bool) -> String {
    let mut output = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(text) => output.push_str(&text.value),
            Inline::Literal { value, .. } => output.push_str(value),
            Inline::Styled { children, .. } => output.push_str(&inline_text_with_attributes(
                children,
                include_attribute_values,
            )),
            Inline::AttributeReference { value, .. } => {
                if include_attribute_values {
                    push_attribute_text(&mut output, value.as_deref().unwrap_or_default());
                }
            }
            Inline::Formula(_) => {}
            Inline::Macro(node) => {
                use crate::inline_model::StandardMacroKind as Kind;
                match node.kind {
                    // A citation carries no readable text of its own: the display
                    // string comes from the host that resolves the key.
                    Kind::Anchor | Kind::BibliographyAnchor | Kind::Citation | Kind::IndexTerm => {}
                    Kind::Email => output.push_str(&node.target),
                    Kind::Footnote
                    | Kind::Keyboard
                    | Kind::Button
                    | Kind::Menu
                    | Kind::Image
                    | Kind::Icon
                    | Kind::Audio
                    | Kind::Video => {
                        if let Some(label) = node.attributes.first() {
                            output.push_str(&label.value);
                        } else {
                            output.push_str(&node.target);
                        }
                    }
                }
            }
            Inline::HardBreak { .. } => output.push('\n'),
            Inline::Passthrough { value, .. } => output.push_str(value),
            Inline::Link(link) => {
                let label = inline_text_with_attributes(&link.label, include_attribute_values);
                output.push_str(if label.is_empty() {
                    &link.target
                } else {
                    &label
                });
            }
            Inline::Reference(reference) => {
                let label = inline_text_with_attributes(&reference.label, include_attribute_values);
                output.push_str(if label.is_empty() {
                    &reference.target_source
                } else {
                    &label
                });
            }
        }
    }
    output
}

fn push_attribute_text(output: &mut String, value: &str) {
    let mut remaining = value;
    while let Some(index) = remaining.find(" +\n") {
        output.push_str(&remaining[..index]);
        output.push('\n');
        remaining = &remaining[index + 3..];
    }
    output.push_str(remaining);
}

fn fold_line_endings(value: &str) -> String {
    value
        .lines()
        .map(|line| line.trim_end_matches([' ', '\t']))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Returns normalized rendering requirements without constructing content
/// projections that the caller does not need.
pub fn rendering_features(analysis: &Analysis) -> RenderingFeatures {
    let mut math_languages = BTreeSet::new();
    let mut source_languages = BTreeSet::new();
    crate::walker::walk(analysis.document(), |node| match node {
        crate::walker::SemanticNode::Inline(Inline::Formula(formula)) => {
            math_languages.insert(math_language_feature(formula.language));
        }
        crate::walker::SemanticNode::Block(AstBlock::Math(formula)) => {
            math_languages.insert(math_language_feature(formula.language));
        }
        crate::walker::SemanticNode::Block(AstBlock::Verbatim(block)) => {
            if let crate::block_model::VerbatimKind::Source(source) = &block.kind
                && let Some(language) = source.language.as_deref()
            {
                source_languages.insert(canonical_source_language(language));
            }
        }
        _ => {}
    });
    RenderingFeatures {
        math_languages: math_languages
            .into_iter()
            .map(|(_, name)| name.to_owned())
            .collect(),
        source_languages: source_languages.into_iter().collect(),
        table_of_contents: analysis.presentation().toc_policy().enabled
            && !analysis.presentation().toc().is_empty(),
    }
}

pub(crate) fn canonical_source_language(language: &str) -> String {
    language
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

const fn math_language_feature(language: crate::inline_model::MathLanguage) -> (u8, &'static str) {
    match language {
        crate::inline_model::MathLanguage::Latex => (0, "latexmath"),
        crate::inline_model::MathLanguage::Typst => (1, "typst"),
    }
}

#[cfg(test)]
mod tests {
    use crate::inline_model::MathLanguage;
    use crate::preprocessor::{
        EffectiveProcessingOptions, PreprocessInputs, PreprocessOptions, ResourceDocument,
        ResourceSnapshot,
    };
    use crate::reference::ResolvedReference;
    use crate::{AnalysisOptions, Engine, SourceId};

    use super::*;
    use crate::core::AnalysisInputs;

    fn analyze_preprocessed_fixture(
        analysis_options: AnalysisOptions,
        source: &str,
        snapshot: &ResourceSnapshot,
        options: &PreprocessOptions,
    ) -> crate::preprocessor::PreprocessedAnalysis {
        EffectiveProcessingOptions::new(analysis_options, options.clone())
            .expect("fixture analysis and preprocess options must match")
            .preprocess_and_analyze(source, snapshot, PreprocessInputs::default())
            .expect("preprocessed analysis")
    }

    #[test]
    fn rendering_features_are_typed_unique_and_deterministically_sorted() {
        let source = include_str!("../../../fixtures/projection/rendering-features.adoc");
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze(source)
            .expect("analysis");
        let projected_formulas = formulas(&analysis);
        assert_eq!(
            projected_formulas
                .iter()
                .map(|formula| formula.kind)
                .collect::<Vec<_>>(),
            [FormulaKind::Inline, FormulaKind::Inline, FormulaKind::Block]
        );

        assert_eq!(
            rendering_features(&analysis),
            RenderingFeatures {
                math_languages: vec!["latexmath".to_owned()],
                source_languages: vec![
                    "c--".to_owned(),
                    "javascript".to_owned(),
                    "rust".to_owned()
                ],
                table_of_contents: true,
            }
        );
        assert!(!analysis.presentation().toc().is_empty());
    }

    #[test]
    fn every_math_language_has_a_stable_rendering_feature_name() {
        assert_eq!(math_language_feature(MathLanguage::Latex), (0, "latexmath"));
        assert_eq!(math_language_feature(MathLanguage::Typst), (1, "typst"));
    }

    #[test]
    fn rendering_features_reflect_preprocessed_includes() {
        let mut snapshot = ResourceSnapshot::default();
        snapshot.insert(
            "features.adoc",
            ResourceDocument {
                source_id: SourceId::new("included:features.adoc"),
                source: include_str!(
                    "../../../fixtures/projection/rendering-features-included.adoc"
                )
                .into(),
            },
        );
        let options = PreprocessOptions {
            enable_includes: true,
            ..PreprocessOptions::default()
        };
        let preprocessed = analyze_preprocessed_fixture(
            AnalysisOptions::default(),
            "include::features.adoc[]\n",
            &snapshot,
            &options,
        );

        assert_eq!(
            rendering_features(&preprocessed.analysis),
            RenderingFeatures {
                math_languages: vec!["latexmath".to_owned()],
                source_languages: vec!["kotlin".to_owned()],
                table_of_contents: false,
            }
        );
    }

    #[test]
    fn rendering_features_are_empty_when_document_needs_no_optional_rendering() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze("Plain paragraph.\n")
            .expect("analysis");

        assert_eq!(rendering_features(&analysis), RenderingFeatures::default());

        let section_without_toc = Engine::new(AnalysisOptions::default())
            .analyze("= Title\n\n== Section\n")
            .expect("analysis");
        assert!(!rendering_features(&section_without_toc).table_of_contents);

        let toc_without_entries = Engine::new(AnalysisOptions::default())
            .analyze("= Title\n:toc:\n")
            .expect("analysis");
        assert!(!rendering_features(&toc_without_entries).table_of_contents);
    }

    #[test]
    fn projections_are_stable_and_keep_links_and_reference_kinds_distinct() {
        let source = "\
= Title

[[part]]
== Section

https://example.com[Site] <<part>> xref:other.adoc#x[] xref:note:42[]

[source,rust]
----
fn main() {}
----

stem:[x+y]
";
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze_with(
                source,
                AnalysisInputs {
                    source_id: Some(&SourceId::new("host:document")),
                    ..AnalysisInputs::default()
                },
            )
            .expect("analysis");
        let projected_links = external_links(&analysis);
        let projected_references = reference_edges(&analysis, &RenderInputs::default());
        let searchable = searchable_text(&analysis);
        let html = crate::html::render(analysis.document(), &crate::html::RenderPolicy::default());

        assert!(html.html.contains("<h1"));
        assert_eq!(
            document_title(&analysis).map(|title| title.text),
            Some("Title".to_owned())
        );
        assert_eq!(projected_links.len(), 1);
        assert_eq!(projected_references.len(), 3);
        assert!(matches!(
            projected_references[0].target,
            ReferenceKey::Local { .. }
        ));
        assert!(matches!(
            projected_references[1].target,
            ReferenceKey::Document { .. }
        ));
        assert!(matches!(
            projected_references[2].target,
            ReferenceKey::Scheme { .. }
        ));
        assert!(searchable.text.contains("fn main() {}"));
        assert!(!searchable.text.contains("x+y"));
    }

    #[test]
    fn block_presentation_titles_use_resolved_inline_text() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze(
                "\
= Title
:product: AdocWeave

.*Important* {product}
[NOTE]
====
body
====
",
            )
            .expect("analysis");
        let projection = block_presentations(&analysis);

        assert_eq!(projection[0].title.as_deref(), Some("Important AdocWeave"));
    }

    #[test]
    fn reference_graph_attaches_optional_resolution_by_exact_source_range() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze("xref:other.adoc[Other]")
            .expect("analysis");
        let resolution =
            ResolvedReference::resolved(analysis.references()[0].range, "https://example/other")
                .with_display_text("Resolved document title");
        let inputs = RenderInputs::default().with_references(vec![resolution]);
        let projected = reference_edges(&analysis, &inputs);

        assert!(matches!(
            projected[0].resolution,
            Some(ResolutionOutcome::Resolved {
                ref href,
                ref display_text,
                ..
            }) if href == "https://example/other"
                && display_text.as_deref() == Some("Resolved document title")
        ));
    }

    #[test]
    fn formula_projection_preserves_inline_and_block_sources() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze(
                "\
stem:[x + y]

[stem]
++++
a^2
++++
",
            )
            .expect("analysis");
        let projected = formulas(&analysis);

        assert_eq!(projected.len(), 2);
        assert_eq!(projected[0].kind, FormulaKind::Inline);
        assert_eq!(projected[0].display(), FormulaKind::Inline);
        assert_eq!(
            projected[0].language,
            crate::inline_model::MathLanguage::Latex
        );
        assert_eq!(projected[0].language.as_asciidoc_name(), "latexmath");
        assert_eq!(projected[0].source, "x + y");
        assert_eq!(
            &analysis.source()[projected[0].content_range.start().to_usize()
                ..projected[0].content_range.end().to_usize()],
            projected[0].source
        );
        assert_eq!(projected[1].kind, FormulaKind::Block);
        assert_eq!(projected[1].display(), FormulaKind::Block);
        assert_eq!(
            projected[1].language,
            crate::inline_model::MathLanguage::Latex
        );
        assert_eq!(projected[1].source, "a^2\n");
        assert_eq!(
            &analysis.source()[projected[1].content_range.start().to_usize()
                ..projected[1].content_range.end().to_usize()],
            projected[1].source
        );
    }

    #[test]
    fn source_block_projection_separates_language_content_and_ranges() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze(
                "\
.main.rs
[source,rust,linenums,start=7]
----
let x = 1;
----
",
            )
            .expect("analysis");
        let projected = source_blocks(&analysis);

        assert_eq!(projected.len(), 1);
        let source = &projected[0];
        assert_eq!(
            source.title.as_ref().map(|title| title.text.as_str()),
            Some("main.rs")
        );
        assert_eq!(source.language.as_deref(), Some("rust"));
        assert!(source.line_numbers);
        assert_eq!(source.start_line, Some(7));
        assert_eq!(source.source, "let x = 1;\n");
        let title = source.title.as_ref().expect("source title");
        let language_range = source.language_range.expect("source language range");
        assert_eq!(
            &analysis.source()
                [source.source_range.start().to_usize()..source.source_range.end().to_usize()],
            "----\nlet x = 1;\n----\n"
        );
        assert!(title.source_range.end() < source.source_range.start());
        assert!(language_range.end() < source.source_range.start());
        assert!(source.source_range.start() <= source.content_range.start());
        assert!(source.content_range.end() <= source.source_range.end());
    }

    #[test]
    fn source_block_line_number_option_spellings_share_one_projection() {
        for attribute in [
            "[source,rust,linenums]",
            "[source,rust,%linenums]",
            "[source,rust,options=linenums]",
        ] {
            let analysis = Engine::new(AnalysisOptions::default())
                .analyze(&format!("{attribute}\n----\ncode\n----\n"))
                .expect("analysis");
            let projected = source_blocks(&analysis);
            let source = &projected[0];

            assert!(source.line_numbers, "{attribute}");
            assert_eq!(source.start_line, Some(1), "{attribute}");
        }
    }

    #[test]
    fn source_block_line_number_boundaries_and_duplicates_are_deterministic() {
        let cases = [
            ("[source,rust,start=8]", false, None),
            ("[source,rust,linenums,start=0]", true, Some(1)),
            ("[source,rust,linenums,start=4294967296]", true, Some(1)),
            ("[source,rust,linenums,start=7,start=9]", true, Some(7)),
            ("[source,rust,linenums,start=0,start=9]", true, Some(1)),
            ("[source,rust,start=8,%linenums]", true, Some(8)),
            ("[source,rust,start=8,options=linenums]", true, Some(8)),
            (
                "[source,rust,%linenums,options=linenums,start=8]",
                true,
                Some(8),
            ),
        ];

        for (attribute, line_numbers, start_line) in cases {
            let analysis = Engine::new(AnalysisOptions::default())
                .analyze(&format!("{attribute}\n----\ncode\n----\n"))
                .expect("analysis");
            let projected = source_blocks(&analysis);
            let source = &projected[0];

            assert_eq!(source.line_numbers, line_numbers, "{attribute}");
            assert_eq!(source.start_line, start_line, "{attribute}");
        }
    }

    #[test]
    fn ordered_list_projection_uses_lowered_presentation() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze(
                "\
[start=4,%reversed,loweralpha]
. one
. two
",
            )
            .expect("analysis");
        let projected = ordered_lists(&analysis);

        assert_eq!(projected.len(), 1);
        assert_eq!(
            projected[0],
            OrderedListProjection {
                source_range: analysis.ast().blocks()[0].range(),
                start: Some(4),
                reversed: true,
                style: crate::block_model::OrderedListStyle::LowerAlpha,
            }
        );
    }

    #[test]
    fn duplicate_resolution_ranges_never_depend_on_input_order() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze("xref:other.adoc[Other]")
            .expect("analysis");
        let range = analysis.references()[0].range;
        let first = ResolvedReference::resolved(range, "https://example/first");
        let second = ResolvedReference::resolved(range, "https://example/second");
        let forward_inputs =
            RenderInputs::default().with_references(vec![first.clone(), second.clone()]);
        let reverse_inputs = RenderInputs::default().with_references(vec![second, first]);
        let forward = reference_edges(&analysis, &forward_inputs);
        let reverse = reference_edges(&analysis, &reverse_inputs);

        assert_eq!(forward, reverse);
        assert!(forward[0].resolution.is_none());
    }

    #[test]
    fn citations_keep_keys_attributes_ranges_and_order() {
        let source = "See cite:[smith2024, tanaka2025] and cite:[a, locator=\"p. 12\"].\n";
        let analysis = crate::Engine::new(crate::AnalysisOptions::default())
            .analyze(source)
            .expect("analysis");
        let value = analysis.citations();
        assert_eq!(value.len(), 2);
        assert_eq!(value[0].order, 0);
        assert_eq!(
            value[0]
                .keys
                .iter()
                .map(|key| key.value.as_str())
                .collect::<Vec<_>>(),
            ["smith2024", "tanaka2025"]
        );
        assert_eq!(value[1].order, 1);
        assert_eq!(value[1].keys[0].value, "a");
        assert_eq!(value[1].attributes[0].name.as_deref(), Some("locator"));
        assert_eq!(value[1].attributes[0].value, "p. 12");
        for citation in &value {
            for key in &citation.keys {
                assert_eq!(
                    &source[key.range.start().to_usize()..key.range.end().to_usize()],
                    key.value
                );
            }
        }
    }

    #[test]
    fn bibliography_catalog_keeps_definition_and_all_reference_ranges() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze("* bibanchor:ref[] Entry\n\nSee <<ref>> and <<ref,Entry>>.")
            .expect("analysis");
        assert_eq!(analysis.catalogs().bibliography().len(), 1);
        assert_eq!(analysis.catalogs().bibliography()[0].references.len(), 2);
    }

    #[test]
    fn bibliography_catalog_keeps_the_anchor_display_text() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze("* [[[smith2024,1]]] Smith, A. 2024.\n\nSee <<smith2024>>.")
            .expect("analysis");
        let entry = &analysis.catalogs().bibliography()[0];
        assert_eq!(entry.id, "smith2024");
        assert_eq!(entry.label.as_deref(), Some("1"));
    }

    #[test]
    fn searchable_text_excludes_attributes_math_and_invisible_anchor_syntax() {
        let source = "\
= Visible
:name: hidden

[[secret]]
== Section

stem:[hidden-math]

....
visible code
....
";
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze(source)
            .expect("analysis");
        let searchable = searchable_text(&analysis);

        assert_eq!(searchable.text, "Visible\nSection\nvisible code");
        assert_eq!(
            searchable
                .segments
                .iter()
                .map(|segment| segment.kind)
                .collect::<Vec<_>>(),
            vec![
                SearchTextKind::Prose,
                SearchTextKind::Prose,
                SearchTextKind::Code
            ]
        );
    }
}
