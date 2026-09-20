//! Block layout for the terminal.

use crate::block_model::{
    AstBlock, AstDocument, BreakBlock, BreakKind, DelimitedContent, DocumentHeader, Heading,
    HeadingKind, ListBlock, LiteralParagraph, Paragraph,
};
use crate::presentation::DocumentPresentation;

use super::inline;
use super::layout::Canvas;
use super::plan::{self, BodyStep};
use super::{TerminalLine, TerminalPolicy, TerminalRole, TerminalStyle};

/// How long a horizontal rule is when the host asked for no width limit.
///
/// A rule has to be some length, and a host that reflows the text still expects
/// a mark it can recognize as a rule.
const UNBOUNDED_RULE_WIDTH: usize = 40;

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
        AstBlock::Verbatim(verbatim) => render_verbatim_text(canvas, &verbatim.value),
        AstBlock::Math(math) => render_verbatim_text(canvas, &math.value),
        AstBlock::List(list) => render_list_items(canvas, list, presentation),
        AstBlock::Delimited(delimited) => match &delimited.content {
            DelimitedContent::Compound(children) => {
                for child in children {
                    render_block(canvas, child, presentation);
                }
            }
            DelimitedContent::Verbatim(value) | DelimitedContent::Passthrough(value) => {
                render_verbatim_text(canvas, value);
            }
            // A table needs its columns measured before it can be drawn.
            DelimitedContent::Table(_) => {}
        },
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
    let mut units = Vec::new();
    // The label of an admonition paragraph is not part of its inline text, so
    // it is written here. Its border and indentation follow with the rest of
    // the admonition presentation.
    if let Some(admonition) = &paragraph.admonition {
        units.extend(inline::plan_text(
            &format!("{}: ", admonition.kind.label()),
            TerminalStyle {
                role: TerminalRole::Admonition(admonition.kind),
                bold: true,
                ..TerminalStyle::default()
            },
        ));
    }
    units.extend(inline::plan(&paragraph.inlines, TerminalStyle::default()));
    canvas.push_wrapped(&units);
}

fn render_literal_paragraph(canvas: &mut Canvas<'_>, literal: &LiteralParagraph) {
    canvas.separate();
    let indent = usize::from(canvas.policy().indent.verbatim);
    canvas.indented(indent, |canvas| {
        for line in literal.value.lines() {
            canvas.push_text(line, TerminalStyle::of(TerminalRole::Code));
        }
    });
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

/// Text that is never reflowed, indented as a block of its own.
///
/// Verbatim, math, and passthrough blocks are shown this way until each of them
/// has a presentation of its own, so nothing an author wrote is missing from
/// the page in the meantime.
fn render_verbatim_text(canvas: &mut Canvas<'_>, value: &str) {
    canvas.separate();
    let indent = usize::from(canvas.policy().indent.verbatim);
    canvas.indented(indent, |canvas| {
        for line in value.lines() {
            canvas.push_text(line, TerminalStyle::of(TerminalRole::Code));
        }
    });
}

/// The items of a list, each as its own block.
///
/// Markers, numbering, and the indentation that shows nesting arrive with the
/// list presentation. Until then an item is shown with a plain marker, so a
/// list still reads as a list.
fn render_list_items(
    canvas: &mut Canvas<'_>,
    list: &ListBlock,
    presentation: &DocumentPresentation,
) {
    for item in &list.items {
        canvas.separate();
        let mut units = inline::plan_text("- ", TerminalStyle::of(TerminalRole::Marker));
        for term in &item.terms {
            units.extend(inline::plan(&term.inlines, TerminalStyle::default()));
            units.extend(inline::plan_text(" ", TerminalStyle::default()));
        }
        units.extend(inline::plan(&item.inlines, TerminalStyle::default()));
        canvas.push_wrapped(&units);
        for continuation in &item.continuations {
            render_block(canvas, continuation, presentation);
        }
        for child in &item.children {
            render_list_items(canvas, child, presentation);
        }
    }
}
