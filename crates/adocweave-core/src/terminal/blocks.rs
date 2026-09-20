//! Block layout for the terminal.

use crate::block_model::{
    AdmonitionKind, AdmonitionPresentation, AstBlock, AstDocument, BlockMetadata, BreakBlock,
    BreakKind, ChecklistState, DelimitedBlock, DelimitedBlockKind, DelimitedContent,
    DelimitedPresentation, DocumentHeader, Heading, HeadingKind, ListBlock, ListItem, ListKind,
    LiteralParagraph, Paragraph, QuoteKind, QuotePresentation, VerbatimBlock, VerbatimKind,
};
use crate::presentation::DocumentPresentation;

use super::inline;
use super::layout::Canvas;
use super::numbering;
use super::plan::{self, BodyStep};
use super::{TerminalLine, TerminalPolicy, TerminalRole, TerminalSpan, TerminalStyle};

/// How long a horizontal rule is when the host asked for no width limit.
///
/// A rule has to be some length, and a host that reflows the text still expects
/// a mark it can recognize as a rule.
const UNBOUNDED_RULE_WIDTH: usize = 40;

/// What every line of a quoted, warned, or verbatim block begins with.
const BORDER: &str = "│ ";

/// Lays a sequence of blocks out on a page of its own. A table cell that holds
/// blocks of its own is rendered this way, into the width of its column.
pub(super) fn render_blocks(
    blocks: &[AstBlock],
    policy: &TerminalPolicy,
    presentation: &DocumentPresentation,
) -> Vec<TerminalLine> {
    let mut canvas = Canvas::new(policy);
    for block in blocks {
        render_block(&mut canvas, block, presentation);
    }
    canvas.finish()
}

pub(super) fn render_document(
    document: &AstDocument,
    policy: &TerminalPolicy,
) -> Vec<TerminalLine> {
    let mut canvas = Canvas::new(policy);
    let presentation = document.presentation();
    for step in plan::body_steps(document, policy) {
        match step {
            BodyStep::Block {
                block,
                header_metadata,
            } => {
                render_block(&mut canvas, block, presentation);
                if header_metadata {
                    render_header_metadata(&mut canvas, document.header());
                }
            }
            // Generated material is placed by the layout and rendered once the
            // table of contents and the footnote list have a presentation.
            BodyStep::TableOfContents | BodyStep::FootnoteCatalog => {}
        }
    }
    canvas.finish()
}

fn render_block(canvas: &mut Canvas<'_>, block: &AstBlock, presentation: &DocumentPresentation) {
    match block {
        AstBlock::Heading(heading) => render_heading(canvas, heading, presentation),
        AstBlock::Paragraph(paragraph) => render_paragraph(canvas, paragraph),
        AstBlock::LiteralParagraph(literal) => render_literal_paragraph(canvas, literal),
        AstBlock::Break(block) => render_break(canvas, block),
        AstBlock::Verbatim(verbatim) => render_verbatim(canvas, verbatim),
        AstBlock::Math(math) => render_verbatim_text(canvas, &math.value, &math.metadata),
        AstBlock::List(list) => render_list(canvas, list, presentation, 0),
        AstBlock::Delimited(delimited) => render_delimited(canvas, delimited, presentation),
        AstBlock::Unsupported(unsupported) => {
            canvas.separate();
            for line in unsupported.raw.lines() {
                canvas.push_text(line, TerminalStyle::of(TerminalRole::Unsupported));
            }
        }
    }
}

fn render_heading(canvas: &mut Canvas<'_>, heading: &Heading, presentation: &DocumentPresentation) {
    // A malformed heading is not a heading. It reads as the text the author
    // typed, which is what the HTML backend shows as well.
    if !heading.well_formed {
        canvas.separate();
        canvas.push_wrapped(&inline::plan(&heading.inlines, TerminalStyle::default()));
        return;
    }

    match heading.kind {
        HeadingKind::DocumentTitle if canvas.policy().render_document_title => {
            canvas.separate();
            let units = inline::plan(
                &heading.inlines,
                TerminalStyle {
                    role: TerminalRole::DocumentTitle,
                    bold: true,
                    ..TerminalStyle::default()
                },
            );
            let columns = inline::units_display_width(&units, canvas.policy().ambiguous_width);
            canvas.push_wrapped(&units);
            canvas.push_rule('═', columns, TerminalStyle::of(TerminalRole::Rule));
        }
        HeadingKind::DocumentTitle => {}
        HeadingKind::Part => render_heading_level(canvas, heading, 1, presentation),
        HeadingKind::Section { level } | HeadingKind::Discrete { level } => {
            render_heading_level(canvas, heading, level, presentation);
        }
    }
}

/// Renders a heading of `level`, where a top-level section and a part are both
/// level 1. Depth is shown by indentation and, at the outermost level, by a
/// rule under the text, so the hierarchy survives without any styling.
fn render_heading_level(
    canvas: &mut Canvas<'_>,
    heading: &Heading,
    level: u8,
    presentation: &DocumentPresentation,
) {
    canvas.separate();
    let step = usize::from(canvas.policy().indent.heading_step);
    let indent = step * usize::from(level.saturating_sub(1));
    let ambiguous = canvas.policy().ambiguous_width;
    canvas.indented(indent, |canvas| {
        let mut units = Vec::new();
        if let Some(heading_presentation) = presentation.heading_at(heading.range)
            && heading_presentation.numbered
            && !heading_presentation.number.is_empty()
        {
            units.extend(inline::plan_text(
                &section_number(&heading_presentation.number),
                TerminalStyle {
                    role: TerminalRole::Marker,
                    bold: true,
                    ..TerminalStyle::default()
                },
            ));
        }
        units.extend(inline::plan(
            &heading.inlines,
            TerminalStyle {
                role: TerminalRole::Heading { level },
                bold: true,
                ..TerminalStyle::default()
            },
        ));
        let columns = inline::units_display_width(&units, ambiguous);
        canvas.push_wrapped(&units);
        if level <= 1 {
            canvas.push_rule('─', columns, TerminalStyle::of(TerminalRole::Rule));
        }
    });
}

fn section_number(number: &[u32]) -> String {
    let mut text = String::new();
    for (index, value) in number.iter().enumerate() {
        if index > 0 {
            text.push('.');
        }
        text.push_str(&value.to_string());
    }
    text.push_str(". ");
    text
}

fn render_header_metadata(canvas: &mut Canvas<'_>, header: &DocumentHeader) {
    let style = TerminalStyle {
        role: TerminalRole::Metadata,
        dim: true,
        ..TerminalStyle::default()
    };
    for author in &header.authors {
        let mut text = author.name.clone();
        if let Some(email) = &author.email {
            text.push_str(" <");
            text.push_str(email);
            text.push('>');
        }
        canvas.push_wrapped(&inline::plan_text(&text, style));
    }
    if let Some(revision) = &header.revision {
        let text = [
            revision.number.as_ref(),
            revision.date.as_ref(),
            revision.remark.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(|value| value.value.as_str())
        .collect::<Vec<_>>()
        .join(" — ");
        if !text.is_empty() {
            canvas.push_wrapped(&inline::plan_text(&text, style));
        }
    }
}

fn render_paragraph(canvas: &mut Canvas<'_>, paragraph: &Paragraph) {
    canvas.separate();
    let units = inline::plan(&paragraph.inlines, TerminalStyle::default());
    match &paragraph.admonition {
        // One paragraph of warning reads best on the same line as its label,
        // with the rest of the text lined up under the first word.
        Some(admonition) => {
            let marker = admonition_marker(admonition.kind);
            canvas.hanging(marker, |canvas| canvas.push_wrapped(&units));
        }
        None => canvas.push_wrapped(&units),
    }
}

fn admonition_marker(kind: AdmonitionKind) -> Vec<TerminalSpan> {
    vec![TerminalSpan::new(
        format!("{}: ", kind.label()),
        TerminalStyle {
            role: TerminalRole::Admonition(kind),
            bold: true,
            ..TerminalStyle::default()
        },
    )]
}

fn render_literal_paragraph(canvas: &mut Canvas<'_>, literal: &LiteralParagraph) {
    render_verbatim_text(canvas, &literal.value, &literal.metadata);
}

fn render_break(canvas: &mut Canvas<'_>, block: &BreakBlock) {
    canvas.separate();
    let columns = canvas.content_width().unwrap_or(UNBOUNDED_RULE_WIDTH);
    let character = match block.kind {
        BreakKind::Thematic => '─',
        BreakKind::Page => '╌',
    };
    canvas.push_rule(character, columns, TerminalStyle::of(TerminalRole::Rule));
}

/// The title an author gave a block, written directly above it.
pub(super) fn render_block_title(canvas: &mut Canvas<'_>, metadata: &BlockMetadata) {
    if let Some(title) = &metadata.title {
        canvas.push_wrapped(&inline::plan(
            &title.inlines,
            TerminalStyle {
                role: TerminalRole::Caption,
                bold: true,
                ..TerminalStyle::default()
            },
        ));
        canvas.keep_tight();
    }
}

fn border(role: TerminalRole, dim: bool) -> Vec<TerminalSpan> {
    vec![TerminalSpan::new(
        BORDER,
        TerminalStyle {
            role,
            dim,
            ..TerminalStyle::default()
        },
    )]
}

/// Text that keeps the lines the author wrote, behind a border that marks it as
/// text to be read exactly as it stands.
fn render_verbatim_text(canvas: &mut Canvas<'_>, value: &str, metadata: &BlockMetadata) {
    canvas.separate();
    render_block_title(canvas, metadata);
    canvas.bordered(border(TerminalRole::Code, true), |canvas| {
        for line in value.lines() {
            canvas.push_text(line, TerminalStyle::of(TerminalRole::Code));
        }
    });
}

/// A listing, literal, or source block. Its lines are never reflowed, because
/// the line breaks are part of what the text means.
fn render_verbatim(canvas: &mut Canvas<'_>, verbatim: &VerbatimBlock) {
    let numbering = match &verbatim.kind {
        VerbatimKind::Source(source) if source.line_numbers => Some(source.start_line.unwrap_or(1)),
        _ => None,
    };
    let Some(first_number) = numbering else {
        render_verbatim_text(canvas, &verbatim.value, &verbatim.metadata);
        return;
    };

    canvas.separate();
    render_block_title(canvas, &verbatim.metadata);
    let lines: Vec<&str> = verbatim.value.lines().collect();
    let last = first_number.saturating_add(u32::try_from(lines.len()).unwrap_or(u32::MAX));
    let width = last.to_string().len();
    let number_style = TerminalStyle {
        role: TerminalRole::Marker,
        dim: true,
        ..TerminalStyle::default()
    };
    for (offset, line) in lines.iter().enumerate() {
        let number = first_number.saturating_add(u32::try_from(offset).unwrap_or(u32::MAX));
        let mut spans = vec![TerminalSpan::new(
            format!("{number:>width$} "),
            number_style,
        )];
        spans.extend(border(TerminalRole::Code, true));
        spans.push(TerminalSpan::new(
            *line,
            TerminalStyle::of(TerminalRole::Code),
        ));
        canvas.push_line(spans);
    }
}

fn render_delimited(
    canvas: &mut Canvas<'_>,
    block: &DelimitedBlock,
    presentation: &DocumentPresentation,
) {
    match &block.presentation {
        Some(DelimitedPresentation::Admonition(admonition)) => {
            render_admonition_block(canvas, block, admonition, presentation);
        }
        Some(DelimitedPresentation::Quote(quote)) => {
            render_quote_block(canvas, block, quote, presentation);
        }
        Some(DelimitedPresentation::Collapsible(_)) => {
            render_collapsible_block(canvas, block, presentation);
        }
        None => match &block.content {
            DelimitedContent::Compound(children) => {
                render_compound_block(canvas, block, children, presentation);
            }
            DelimitedContent::Verbatim(value) => {
                // A comment block is written for the author, not the reader.
                if block.kind != DelimitedBlockKind::Comment {
                    render_verbatim_text(canvas, value, &block.metadata);
                }
            }
            DelimitedContent::Passthrough(value) => {
                canvas.separate();
                for line in value.lines() {
                    canvas.push_text(line, TerminalStyle::of(TerminalRole::Muted));
                }
            }
            DelimitedContent::Table(table) => {
                super::table::render(canvas, table, &block.metadata, presentation);
            }
        },
    }
}

fn render_delimited_children(
    canvas: &mut Canvas<'_>,
    block: &DelimitedBlock,
    presentation: &DocumentPresentation,
) {
    match &block.content {
        DelimitedContent::Compound(children) => {
            for child in children {
                render_block(canvas, child, presentation);
            }
        }
        DelimitedContent::Verbatim(value) | DelimitedContent::Passthrough(value) => {
            for line in value.lines() {
                canvas.push_text(line, TerminalStyle::of(TerminalRole::Code));
            }
        }
        DelimitedContent::Table(_) => {}
    }
}

/// A warning that spans several blocks: its kind is named on a line of its own,
/// and a border runs down everything it covers.
fn render_admonition_block(
    canvas: &mut Canvas<'_>,
    block: &DelimitedBlock,
    admonition: &AdmonitionPresentation,
    presentation: &DocumentPresentation,
) {
    canvas.separate();
    canvas.push_line(vec![TerminalSpan::new(
        admonition.kind.label(),
        TerminalStyle {
            role: TerminalRole::Admonition(admonition.kind),
            bold: true,
            ..TerminalStyle::default()
        },
    )]);
    canvas.bordered(
        border(TerminalRole::Admonition(admonition.kind), false),
        |canvas| {
            render_block_title(canvas, &block.metadata);
            render_delimited_children(canvas, block, presentation);
        },
    );
}

fn render_quote_block(
    canvas: &mut Canvas<'_>,
    block: &DelimitedBlock,
    quote: &QuotePresentation,
    presentation: &DocumentPresentation,
) {
    canvas.separate();
    canvas.bordered(border(TerminalRole::Quote, false), |canvas| {
        render_block_title(canvas, &block.metadata);
        match quote.kind {
            // A verse keeps the line breaks the poet wrote.
            QuoteKind::Verse => render_verse(canvas, block, presentation),
            QuoteKind::Quote => render_delimited_children(canvas, block, presentation),
        }
        if let Some(attribution) = attribution_text(quote) {
            canvas.separate();
            canvas.push_wrapped(&inline::plan_text(
                &attribution,
                TerminalStyle::of(TerminalRole::Attribution),
            ));
        }
    });
}

fn attribution_text(quote: &QuotePresentation) -> Option<String> {
    let mut text = String::new();
    if let Some(attribution) = &quote.attribution {
        text.push_str("— ");
        text.push_str(&attribution.value);
    }
    if let Some(citation) = &quote.citation {
        if text.is_empty() {
            text.push_str("— ");
        } else {
            text.push_str(", ");
        }
        text.push_str(&citation.value);
    }
    (!text.is_empty()).then_some(text)
}

fn render_verse(
    canvas: &mut Canvas<'_>,
    block: &DelimitedBlock,
    presentation: &DocumentPresentation,
) {
    let DelimitedContent::Compound(children) = &block.content else {
        render_delimited_children(canvas, block, presentation);
        return;
    };
    for child in children {
        let AstBlock::Paragraph(paragraph) = child else {
            render_block(canvas, child, presentation);
            continue;
        };
        canvas.separate();
        for line in paragraph.value.lines() {
            canvas.push_text(line, TerminalStyle::default());
        }
    }
}

/// A collapsible block is always open here, because a terminal page cannot be
/// unfolded. The mark in front of the title says that it is a disclosure.
fn render_collapsible_block(
    canvas: &mut Canvas<'_>,
    block: &DelimitedBlock,
    presentation: &DocumentPresentation,
) {
    canvas.separate();
    let title = match &block.metadata.title {
        Some(title) => inline::plan(
            &title.inlines,
            TerminalStyle {
                role: TerminalRole::Caption,
                bold: true,
                ..TerminalStyle::default()
            },
        ),
        None => inline::plan_text(
            "Details",
            TerminalStyle {
                role: TerminalRole::Caption,
                bold: true,
                ..TerminalStyle::default()
            },
        ),
    };
    canvas.hanging(
        vec![TerminalSpan::new(
            "▾ ",
            TerminalStyle::of(TerminalRole::Marker),
        )],
        |canvas| {
            canvas.push_wrapped(&title);
            canvas.keep_tight();
            render_delimited_children(canvas, block, presentation);
        },
    );
}

/// An example or a sidebar is a container the reader can see, so it is framed.
/// An open block is transparent and adds nothing of its own.
fn render_compound_block(
    canvas: &mut Canvas<'_>,
    block: &DelimitedBlock,
    children: &[AstBlock],
    presentation: &DocumentPresentation,
) {
    let framed = matches!(
        block.kind,
        DelimitedBlockKind::Example | DelimitedBlockKind::Sidebar
    );
    if !framed {
        for child in children {
            render_block(canvas, child, presentation);
        }
        return;
    }

    canvas.separate();
    let columns = canvas.content_width().unwrap_or(UNBOUNDED_RULE_WIDTH);
    let rule = TerminalStyle::of(TerminalRole::Rule);
    render_block_title(canvas, &block.metadata);
    canvas.push_rule('─', columns, rule);
    canvas.keep_tight();
    for child in children {
        render_block(canvas, child, presentation);
    }
    canvas.push_rule('─', columns, rule);
}

fn render_list(
    canvas: &mut Canvas<'_>,
    list: &ListBlock,
    presentation: &DocumentPresentation,
    depth: usize,
) {
    // A list stands apart from the text around it, but a list nested in an
    // item belongs to that item and follows it directly.
    if depth == 0 {
        canvas.separate();
    }
    let markers = item_markers(canvas, list, depth);
    for (item, marker) in list.items.iter().zip(markers) {
        match (list.kind, item.terms.is_empty()) {
            // A term and what it means are two different things, so the term
            // stands on its own line and the description sits under it.
            (ListKind::Description, false) => {
                render_description_item(canvas, item, presentation, depth);
            }
            _ => canvas.hanging(marker, |canvas| {
                render_item_body(canvas, item, presentation, depth);
            }),
        }
    }
}

fn render_description_item(
    canvas: &mut Canvas<'_>,
    item: &ListItem,
    presentation: &DocumentPresentation,
    depth: usize,
) {
    for term in &item.terms {
        canvas.push_wrapped(&inline::plan(
            &term.inlines,
            TerminalStyle {
                bold: true,
                ..TerminalStyle::default()
            },
        ));
    }
    let nested = usize::from(canvas.policy().indent.nested);
    canvas.indented(nested, |canvas| {
        render_item_body(canvas, item, presentation, depth);
    });
}

fn render_item_body(
    canvas: &mut Canvas<'_>,
    item: &ListItem,
    presentation: &DocumentPresentation,
    depth: usize,
) {
    let mut units = Vec::new();
    if let Some(state) = item.checklist {
        let markers = canvas.policy().list_markers.checklist;
        let box_text = match state {
            ChecklistState::Unchecked => markers[0],
            ChecklistState::Checked => markers[1],
        };
        units.extend(inline::plan_text(
            &format!("{box_text} "),
            TerminalStyle::of(TerminalRole::Marker),
        ));
    }
    units.extend(inline::plan(&item.inlines, TerminalStyle::default()));
    if !units.is_empty() {
        canvas.push_wrapped(&units);
    }
    for continuation in &item.continuations {
        render_block(canvas, continuation, presentation);
    }
    for child in &item.children {
        render_list(canvas, child, presentation, depth + 1);
    }
}

/// The marker in front of every item, already padded so the text of each item
/// starts in the same column.
fn item_markers(canvas: &Canvas<'_>, list: &ListBlock, depth: usize) -> Vec<Vec<TerminalSpan>> {
    let style = TerminalStyle::of(TerminalRole::Marker);
    let labels: Vec<String> = match list.kind {
        ListKind::Ordered => {
            let separator = canvas.policy().list_markers.ordered_separator;
            numbering::item_numbers(list)
                .into_iter()
                .map(|number| {
                    format!(
                        "{}{separator}",
                        numbering::label(number, list.presentation.style)
                    )
                })
                .collect()
        }
        ListKind::Callout => list
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let number = item
                    .callout_id
                    .unwrap_or_else(|| u32::try_from(index + 1).unwrap_or(u32::MAX));
                format!("<{number}>")
            })
            .collect(),
        ListKind::Unordered | ListKind::Description => {
            let markers = canvas.policy().list_markers.unordered;
            let marker = markers[depth % markers.len()];
            list.items.iter().map(|_| marker.to_string()).collect()
        }
    };
    let width = labels
        .iter()
        .map(|label| super::display_width(label, canvas.policy().ambiguous_width))
        .max()
        .unwrap_or(0);
    labels
        .into_iter()
        .map(|label| {
            let padding = width - super::display_width(&label, canvas.policy().ambiguous_width);
            vec![TerminalSpan::new(
                format!("{}{label} ", " ".repeat(padding)),
                style,
            )]
        })
        .collect()
}
