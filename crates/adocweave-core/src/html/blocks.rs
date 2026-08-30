//! Block-level HTML rendering over the planned body writer.

use super::*;

pub(super) fn serialize_body_traversal(
    output: &mut String,
    document: &AstDocument,
    plan: &body::BodyTraversalPlan<'_>,
    policy: &RenderPolicy,
    context: &mut InlineRenderContext<'_, '_>,
) {
    for step in &plan.steps {
        match step {
            body::BodyTraversalStep::TableOfContents => {
                render_toc(output, document.presentation());
            }
            body::BodyTraversalStep::FootnoteCatalog => {
                render_footnote_catalog(output, document.catalogs(), document.facts(), context);
            }
            body::BodyTraversalStep::Block {
                block,
                scope,
                render_header_metadata: include_header_metadata,
            } => {
                render_block(output, block, policy, context, *scope);
                if *include_header_metadata {
                    render_header_metadata(output, document.header());
                }
            }
        }
    }
}

pub(super) fn render_header_metadata(
    output: &mut String,
    header: &crate::block_model::DocumentHeader,
) {
    for author in &header.authors {
        BlockWriter::start(output, "p", &[classes(&["author"])]);
        BlockWriter::text(output, &author.name);
        if let Some(email) = &author.email {
            BlockWriter::text(output, " <");
            BlockWriter::text(output, email);
            BlockWriter::text(output, ">");
        }
        BlockWriter::end(output, "p");
        BlockWriter::line_break(output);
    }
    if let Some(revision) = &header.revision {
        BlockWriter::start(output, "p", &[classes(&["revision"])]);
        let mut separator = "";
        for value in [
            revision.number.as_ref(),
            revision.date.as_ref(),
            revision.remark.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            BlockWriter::text(output, separator);
            BlockWriter::text(output, &value.value);
            separator = " — ";
        }
        BlockWriter::end(output, "p");
        BlockWriter::line_break(output);
    }
}

pub(super) fn render_block(
    output: &mut String,
    block: &AstBlock,
    policy: &RenderPolicy,
    context: &mut InlineRenderContext<'_, '_>,
    scope: RenderScope,
) {
    let explicit_id = context
        .identifiers
        .target_at(block.range())
        .map(|target| target.id.as_str());
    match block {
        AstBlock::Heading(heading) => {
            let id = if let Some(id) = context
                .identifiers
                .heading_at(heading.text_range)
                .map(|heading| heading.id.as_str())
            {
                id
            } else if let Some(id) = explicit_id {
                id
            } else {
                unreachable!("lowering assigns every heading an identifier")
            };
            render_heading(output, heading, id, policy, context);
        }
        AstBlock::Paragraph(paragraph) => {
            if let Some(admonition) = &paragraph.admonition {
                render_admonition_start(
                    output,
                    admonition,
                    explicit_id,
                    &paragraph.metadata,
                    context,
                );
                render_paragraph(output, paragraph, None, context);
                BlockWriter::end(output, "div");
                BlockWriter::line_break(output);
            } else if crate::caption::block_image(paragraph).is_some() {
                render_image_block(output, paragraph, explicit_id, context);
            } else {
                render_paragraph(output, paragraph, explicit_id, context);
            }
        }
        AstBlock::LiteralParagraph(paragraph) => {
            let attributes = block_attributes(explicit_id, &paragraph.metadata, &[], context);
            render_preformatted(output, &attributes, &paragraph.value);
        }
        AstBlock::Break(block) => render_break(output, block, explicit_id, context),
        AstBlock::Verbatim(block) => match &block.kind {
            crate::block_model::VerbatimKind::Source(source) => {
                let has_presentation = block.metadata.title.is_some() || source.line_numbers;
                if has_presentation {
                    let attributes =
                        block_attributes(explicit_id, &block.metadata, &["source-block"], context);
                    BlockWriter::start(output, "figure", &attributes);
                    BlockWriter::line_break(output);
                    if let Some(title) = &block.metadata.title {
                        render_verbatim_caption(output, title, block.range, context);
                    }
                }
                let mut pre_attributes = Vec::new();
                if !has_presentation {
                    pre_attributes.extend(block_attributes(
                        explicit_id,
                        &block.metadata,
                        &[],
                        context,
                    ));
                }
                if has_presentation
                    && let Some(language) = &source.language
                    && policy.source_languages.allows(language)
                {
                    pre_attributes.push(passive("data-language", language));
                }
                if source.line_numbers {
                    pre_attributes.push(passive("data-line-numbers", "true"));
                    pre_attributes.push(passive(
                        "data-line-start",
                        source.start_line.unwrap_or(1).to_string(),
                    ));
                }
                BlockWriter::start(output, "pre", &pre_attributes);
                let mut code_attributes = Vec::new();
                if let Some(language) = &source.language {
                    if policy.source_languages.allows(language) {
                        code_attributes.push(source_language_class(language));
                    } else if policy.source_languages.unknown == UnknownSourceLanguage::Diagnostic {
                        context.diagnostics.push(render_diagnostic(
                            "source-language-not-allowed",
                            "source language is rejected by the render policy",
                            source.language_range.unwrap_or(source.attribute_range),
                        ));
                    }
                }
                BlockWriter::start(output, "code", &code_attributes);
                BlockWriter::text(output, &block.value);
                BlockWriter::end(output, "code");
                BlockWriter::end(output, "pre");
                BlockWriter::line_break(output);
                if has_presentation {
                    BlockWriter::end(output, "figure");
                    BlockWriter::line_break(output);
                }
            }
            crate::block_model::VerbatimKind::Listing
            | crate::block_model::VerbatimKind::Literal => {
                // A titled listing or literal block is a figure like a titled
                // source block, so the title is not lost; a listing title
                // carries the listing caption number when the document sets it.
                if let Some(title) = &block.metadata.title {
                    let class = match block.kind {
                        crate::block_model::VerbatimKind::Listing => "listing-block",
                        _ => "literal-block",
                    };
                    let attributes =
                        block_attributes(explicit_id, &block.metadata, &[class], context);
                    BlockWriter::start(output, "figure", &attributes);
                    BlockWriter::line_break(output);
                    render_verbatim_caption(output, title, block.range, context);
                    render_preformatted(output, &[], &block.value);
                    BlockWriter::end(output, "figure");
                    BlockWriter::line_break(output);
                } else {
                    let attributes = block_attributes(explicit_id, &block.metadata, &[], context);
                    render_preformatted(output, &attributes, &block.value);
                }
            }
        },
        AstBlock::List(list) => render_list(output, list, explicit_id, policy, context, scope),
        AstBlock::Math(block) => {
            if policy.math_languages.allowed.contains(&block.language) {
                let mut attributes = block_attributes(
                    explicit_id,
                    &block.metadata,
                    &[body::math_class(block.language)],
                    context,
                );
                attributes.extend(body::math_data_attributes(block.language, "block"));
                BlockWriter::start(output, "pre", &attributes);
                BlockWriter::start(output, "code", &[]);
                BlockWriter::text(output, &block.value);
                BlockWriter::end(output, "code");
                BlockWriter::end(output, "pre");
                BlockWriter::line_break(output);
            } else {
                let attributes = block_attributes(explicit_id, &block.metadata, &[], context);
                render_preformatted(output, &attributes, &block.value);
                context.diagnostics.push(render_diagnostic(
                    "math-language-not-allowed",
                    "math language is rejected by the render policy",
                    block.attribute_range,
                ));
            }
        }
        AstBlock::Delimited(block) => {
            render_delimited(output, block, explicit_id, policy, context, scope);
        }
        AstBlock::Unsupported(block) => render_unsupported(output, block, explicit_id),
    }
}

pub(super) fn render_preformatted(
    output: &mut String,
    attributes: &[body::PlannedAttribute],
    value: &str,
) {
    BlockWriter::start(output, "pre", attributes);
    BlockWriter::text(output, value);
    BlockWriter::end(output, "pre");
    BlockWriter::line_break(output);
}

pub(super) fn render_delimited(
    output: &mut String,
    block: &crate::block_model::DelimitedBlock,
    explicit_id: Option<&str>,
    policy: &RenderPolicy,
    context: &mut InlineRenderContext<'_, '_>,
    scope: RenderScope,
) {
    if let Some(presentation) = &block.presentation {
        match presentation {
            crate::block_model::DelimitedPresentation::Admonition(admonition) => {
                render_admonition_start(output, admonition, explicit_id, &block.metadata, context);
                render_delimited_children(output, block, policy, context, scope);
                BlockWriter::end(output, "div");
                BlockWriter::line_break(output);
                return;
            }
            crate::block_model::DelimitedPresentation::Quote(quote) => {
                let quote_class = match quote.kind {
                    crate::block_model::QuoteKind::Quote => "quote",
                    crate::block_model::QuoteKind::Verse => "verse",
                };
                let attributes =
                    block_attributes(explicit_id, &block.metadata, &[quote_class], context);
                BlockWriter::start(output, "div", &attributes);
                BlockWriter::line_break(output);
                if quote.kind == crate::block_model::QuoteKind::Quote {
                    BlockWriter::start(output, "blockquote", &[]);
                    BlockWriter::line_break(output);
                }
                if quote.kind == crate::block_model::QuoteKind::Verse {
                    render_verse_children(output, block, policy, context, scope);
                } else {
                    render_delimited_children(output, block, policy, context, scope);
                }
                if quote.kind == crate::block_model::QuoteKind::Quote {
                    BlockWriter::end(output, "blockquote");
                    BlockWriter::line_break(output);
                }
                if quote.attribution.is_some() || quote.citation.is_some() {
                    BlockWriter::start(output, "div", &[classes(&["attribution"])]);
                    if let Some(attribution) = &quote.attribution {
                        BlockWriter::text(output, "— ");
                        BlockWriter::text(output, &attribution.value);
                    }
                    if let Some(citation) = &quote.citation {
                        BlockWriter::text(output, " ");
                        BlockWriter::start(output, "cite", &[]);
                        BlockWriter::text(output, &citation.value);
                        BlockWriter::end(output, "cite");
                    }
                    BlockWriter::end(output, "div");
                    BlockWriter::line_break(output);
                }
                BlockWriter::end(output, "div");
                BlockWriter::line_break(output);
                return;
            }
            crate::block_model::DelimitedPresentation::Collapsible(collapsible) => {
                // A disclosure: the title is what the reader clicks. Without a
                // title the language's default word stands in.
                let mut attributes = block_attributes(explicit_id, &block.metadata, &[], context);
                if collapsible.open {
                    attributes.push(body::boolean("open"));
                }
                BlockWriter::start(output, "details", &attributes);
                BlockWriter::line_break(output);
                BlockWriter::start(output, "summary", &[]);
                if let Some(title) = &block.metadata.title {
                    render_inlines(output, &title.inlines, context);
                } else {
                    BlockWriter::text(output, "Details");
                }
                BlockWriter::end(output, "summary");
                BlockWriter::line_break(output);
                render_delimited_children(output, block, policy, context, scope);
                BlockWriter::end(output, "details");
                BlockWriter::line_break(output);
                return;
            }
        }
    }
    match &block.content {
        crate::block_model::DelimitedContent::Verbatim(value) => {
            if !matches!(block.kind, crate::block_model::DelimitedBlockKind::Comment) {
                let attributes = block_attributes(explicit_id, &block.metadata, &[], context);
                render_preformatted(output, &attributes, value);
            }
        }
        crate::block_model::DelimitedContent::Passthrough(value) => {
            let attributes = block_attributes(explicit_id, &block.metadata, &[], context);
            render_preformatted(output, &attributes, value);
        }
        crate::block_model::DelimitedContent::Table(table) => {
            render_table(output, table, block, explicit_id, policy, context, scope);
        }
        crate::block_model::DelimitedContent::Compound(_) => {
            render_compound(output, block, explicit_id, policy, context, scope);
        }
    }
}

/// A compound block is a container the reader can see: the wrapper names its
/// kind, and the block title leads the content the same way it does for an
/// admonition. A quote delimiter without a quote style stays transparent, as
/// the compatibility guide records.
fn render_compound(
    output: &mut String,
    block: &crate::block_model::DelimitedBlock,
    explicit_id: Option<&str>,
    policy: &RenderPolicy,
    context: &mut InlineRenderContext<'_, '_>,
    scope: RenderScope,
) {
    let kind_class = match block.kind {
        crate::block_model::DelimitedBlockKind::Example => "example",
        crate::block_model::DelimitedBlockKind::Sidebar => "sidebar",
        crate::block_model::DelimitedBlockKind::Open => "open",
        _ => {
            render_delimited_children(output, block, policy, context, scope);
            return;
        }
    };
    let attributes = block_attributes(explicit_id, &block.metadata, &[kind_class], context);
    BlockWriter::start(output, "div", &attributes);
    BlockWriter::line_break(output);
    if let Some(title) = &block.metadata.title {
        BlockWriter::start(output, "div", &[classes(&["title"])]);
        if let Some(lead) = caption_lead(block.range, context) {
            BlockWriter::text(output, &lead);
        }
        render_inlines(output, &title.inlines, context);
        BlockWriter::end(output, "div");
        BlockWriter::line_break(output);
    }
    render_delimited_children(output, block, policy, context, scope);
    BlockWriter::end(output, "div");
    BlockWriter::line_break(output);
}

pub(super) fn render_delimited_children(
    output: &mut String,
    block: &crate::block_model::DelimitedBlock,
    policy: &RenderPolicy,
    context: &mut InlineRenderContext<'_, '_>,
    scope: RenderScope,
) {
    if let crate::block_model::DelimitedContent::Compound(children) = &block.content {
        for child in children {
            render_block(output, child, policy, context, scope);
        }
    }
}

pub(super) fn render_verse_children(
    output: &mut String,
    block: &crate::block_model::DelimitedBlock,
    policy: &RenderPolicy,
    context: &mut InlineRenderContext<'_, '_>,
    scope: RenderScope,
) {
    let crate::block_model::DelimitedContent::Compound(children) = &block.content else {
        return;
    };
    if children
        .iter()
        .all(|child| matches!(child, AstBlock::Paragraph(_)))
    {
        BlockWriter::start(output, "pre", &[]);
        for (index, child) in children.iter().enumerate() {
            let AstBlock::Paragraph(paragraph) = child else {
                unreachable!()
            };
            if index > 0 {
                BlockWriter::line_break(output);
                BlockWriter::line_break(output);
            }
            // Verse preserves source line boundaries. Rendering the stored source text
            // avoids the normal paragraph inline renderer's intentional newline folding.
            BlockWriter::text(output, &paragraph.value);
        }
        BlockWriter::end(output, "pre");
        BlockWriter::line_break(output);
    } else {
        render_delimited_children(output, block, policy, context, scope);
    }
}

pub(super) fn render_admonition_start(
    output: &mut String,
    admonition: &crate::block_model::AdmonitionPresentation,
    explicit_id: Option<&str>,
    metadata: &crate::block_model::BlockMetadata,
    context: &mut InlineRenderContext<'_, '_>,
) {
    let kind_class = match admonition.kind.label() {
        "CAUTION" => "admonition-caution",
        "IMPORTANT" => "admonition-important",
        "NOTE" => "admonition-note",
        "TIP" => "admonition-tip",
        "WARNING" => "admonition-warning",
        _ => unreachable!("admonition kinds have fixed labels"),
    };
    let attributes = block_attributes(explicit_id, metadata, &["admonition", kind_class], context);
    BlockWriter::start(output, "div", &attributes);
    BlockWriter::start(output, "div", &[classes(&["title"])]);
    if let Some(title) = &metadata.title {
        render_inlines(output, &title.inlines, context);
    } else {
        BlockWriter::text(output, admonition.kind.label());
    }
    BlockWriter::end(output, "div");
    BlockWriter::line_break(output);
}

pub(super) fn render_table(
    output: &mut String,
    table: &crate::table::Table,
    block: &crate::block_model::DelimitedBlock,
    explicit_id: Option<&str>,
    policy: &RenderPolicy,
    context: &mut InlineRenderContext<'_, '_>,
    scope: RenderScope,
) {
    let metadata = &block.metadata;
    let caption_lead = caption_lead(block.range, context);
    use crate::table::{
        HorizontalAlignment, TableCellStyle, TableFrame, TableGrid, TableSection, TableStripes,
    };
    let frame_class = match table.presentation.frame {
        TableFrame::All => "table-frame-all",
        TableFrame::Ends => "table-frame-ends",
        TableFrame::None => "table-frame-none",
        TableFrame::Sides => "table-frame-sides",
    };
    let grid_class = match table.presentation.grid {
        TableGrid::All => "table-grid-all",
        TableGrid::Columns => "table-grid-cols",
        TableGrid::None => "table-grid-none",
        TableGrid::Rows => "table-grid-rows",
    };
    let stripes_class = match table.presentation.stripes {
        TableStripes::All => "table-stripes-all",
        TableStripes::Even => "table-stripes-even",
        TableStripes::Hover => "table-stripes-hover",
        TableStripes::None => "table-stripes-none",
        TableStripes::Odd => "table-stripes-odd",
    };
    let mut attributes = block_attributes(
        explicit_id,
        metadata,
        &[frame_class, grid_class, stripes_class],
        context,
    );
    if let Some(width) = table.presentation.width {
        attributes.push(passive("width", format!("{width}%")));
    }
    BlockWriter::start(output, "table", &attributes);
    BlockWriter::line_break(output);
    if let Some(caption) = &metadata.title {
        BlockWriter::start(output, "caption", &[]);
        if let Some(lead) = &caption_lead {
            BlockWriter::text(output, lead);
        }
        render_inlines(output, &caption.inlines, context);
        BlockWriter::end(output, "caption");
        BlockWriter::line_break(output);
    }
    let mut section = None;
    for row in &table.rows {
        if section != Some(row.section) {
            if let Some(previous) = section {
                BlockWriter::end(output, table_section_name(previous));
                BlockWriter::line_break(output);
            }
            BlockWriter::start(output, table_section_name(row.section), &[]);
            BlockWriter::line_break(output);
            section = Some(row.section);
        }
        BlockWriter::start(output, "tr", &[]);
        BlockWriter::line_break(output);
        for cell in &row.cells {
            let tag = if row.section == TableSection::Header || cell.style == TableCellStyle::Header
            {
                "th"
            } else {
                "td"
            };
            let mut cell_attributes = Vec::new();
            if cell.column_span > 1 {
                cell_attributes.push(passive("colspan", cell.column_span.to_string()));
            }
            if cell.row_span > 1 {
                cell_attributes.push(passive("rowspan", cell.row_span.to_string()));
            }
            let alignment = cell.horizontal_alignment.unwrap_or_else(|| {
                table
                    .columns
                    .get(cell.column_index as usize)
                    .map_or(HorizontalAlignment::Left, |column| {
                        column.horizontal_alignment
                    })
            });
            let vertical_alignment = cell.vertical_alignment.unwrap_or_else(|| {
                table
                    .columns
                    .get(cell.column_index as usize)
                    .map_or(crate::table::VerticalAlignment::Top, |column| {
                        column.vertical_alignment
                    })
            });
            let horizontal_class = match alignment {
                HorizontalAlignment::Left => "table-align-left",
                HorizontalAlignment::Center => "table-align-center",
                HorizontalAlignment::Right => "table-align-right",
            };
            let vertical_class = match vertical_alignment {
                crate::table::VerticalAlignment::Top => "table-valign-top",
                crate::table::VerticalAlignment::Middle => "table-valign-middle",
                crate::table::VerticalAlignment::Bottom => "table-valign-bottom",
            };
            cell_attributes.push(classes(&[horizontal_class, vertical_class]));
            BlockWriter::start(output, tag, &cell_attributes);
            render_table_cell(output, cell, policy, context, scope);
            BlockWriter::end(output, tag);
            BlockWriter::line_break(output);
        }
        BlockWriter::end(output, "tr");
        BlockWriter::line_break(output);
    }
    if let Some(section) = section {
        BlockWriter::end(output, table_section_name(section));
        BlockWriter::line_break(output);
    }
    BlockWriter::end(output, "table");
    BlockWriter::line_break(output);
}

pub(super) fn table_section_name(section: crate::table::TableSection) -> &'static str {
    match section {
        crate::table::TableSection::Header => "thead",
        crate::table::TableSection::Body => "tbody",
        crate::table::TableSection::Footer => "tfoot",
    }
}

pub(super) fn render_table_cell(
    output: &mut String,
    cell: &crate::table::TableCell,
    policy: &RenderPolicy,
    context: &mut InlineRenderContext<'_, '_>,
    scope: RenderScope,
) {
    use crate::table::{TableCellContent, TableCellStyle};
    match &cell.content {
        TableCellContent::Verbatim(value) => {
            BlockWriter::start(output, "pre", &[]);
            BlockWriter::text(output, value);
            BlockWriter::end(output, "pre");
        }
        TableCellContent::Inlines(inlines) => {
            let wrapper = match cell.style {
                TableCellStyle::Emphasis => Some("em"),
                TableCellStyle::Monospace => Some("code"),
                TableCellStyle::Strong => Some("strong"),
                _ => None,
            };
            if let Some(wrapper) = wrapper {
                BlockWriter::start(output, wrapper, &[]);
            }
            render_inlines(output, inlines, context);
            if let Some(wrapper) = wrapper {
                BlockWriter::end(output, wrapper);
            }
        }
        TableCellContent::AsciiDoc(blocks) => {
            for block in blocks {
                render_block(output, block, policy, context, scope);
            }
        }
    }
}

pub(super) fn render_break(
    output: &mut String,
    block: &crate::block_model::BreakBlock,
    id: Option<&str>,
    context: &mut InlineRenderContext<'_, '_>,
) {
    let fixed: &[&'static str] = if block.kind == crate::block_model::BreakKind::Page {
        &["page-break"]
    } else {
        &[]
    };
    let attributes = block_attributes(id, &block.metadata, fixed, context);
    BlockWriter::void(output, "hr", &attributes);
    BlockWriter::line_break(output);
}

pub(super) fn render_list(
    output: &mut String,
    list: &crate::block_model::ListBlock,
    explicit_id: Option<&str>,
    policy: &RenderPolicy,
    context: &mut InlineRenderContext<'_, '_>,
    scope: RenderScope,
) {
    let tag = match list.kind {
        crate::block_model::ListKind::Unordered => "ul",
        crate::block_model::ListKind::Ordered => "ol",
        crate::block_model::ListKind::Description => "dl",
        crate::block_model::ListKind::Callout => "ol",
    };
    let fixed: &[&'static str] = if list.kind == crate::block_model::ListKind::Callout {
        &["callout-list"]
    } else {
        &[]
    };
    let attributes = block_attributes(explicit_id, &list.metadata, fixed, context);
    BlockWriter::start(output, tag, &attributes);
    BlockWriter::line_break(output);
    for item in &list.items {
        if list.kind == crate::block_model::ListKind::Description {
            for term in &item.terms {
                BlockWriter::start(output, "dt", &[]);
                render_inlines(output, &term.inlines, context);
                BlockWriter::end(output, "dt");
                BlockWriter::line_break(output);
            }
            BlockWriter::start(output, "dd", &[]);
        } else {
            BlockWriter::start(output, "li", &[]);
        }
        if let Some(state) = item.checklist {
            BlockWriter::start(output, "span", &[classes(&["checklist-marker"])]);
            BlockWriter::text(
                output,
                if state == crate::block_model::ChecklistState::Checked {
                    "☑"
                } else {
                    "☐"
                },
            );
            BlockWriter::end(output, "span");
            BlockWriter::text(output, " ");
        }
        if let Some(id) = item.callout_id {
            BlockWriter::start(output, "span", &[classes(&["callout-number"])]);
            BlockWriter::text(output, &id.to_string());
            BlockWriter::end(output, "span");
            BlockWriter::text(output, " ");
        }
        render_inlines(output, &item.inlines, context);
        if scope.bibliography_section
            && list.kind == crate::block_model::ListKind::Unordered
            && let Some(entry) = bibliography_entry_for_item(&item.inlines, context.catalogs)
        {
            render_bibliography_backrefs(output, entry);
        }
        for child in &item.children {
            BlockWriter::line_break(output);
            render_list(output, child, None, policy, context, scope);
        }
        for continuation in &item.continuations {
            if !output.ends_with('\n') {
                BlockWriter::line_break(output);
            }
            render_block(output, continuation, policy, context, scope);
        }
        BlockWriter::end(
            output,
            if list.kind == crate::block_model::ListKind::Description {
                "dd"
            } else {
                "li"
            },
        );
        BlockWriter::line_break(output);
    }
    BlockWriter::end(output, tag);
    BlockWriter::line_break(output);
}

pub(super) fn bibliography_entry_for_item<'a>(
    inlines: &[Inline],
    catalogs: &'a crate::catalog::DocumentCatalogs,
) -> Option<&'a crate::catalog::BibliographyEntry> {
    inlines.iter().find_map(|inline| {
        let Inline::Macro(node) = inline else {
            return None;
        };
        (node.kind == crate::inline_model::StandardMacroKind::BibliographyAnchor)
            .then(|| {
                catalogs
                    .bibliography()
                    .iter()
                    .find(|entry| entry.definition_range == node.range)
            })
            .flatten()
    })
}

pub(super) fn bibliography_reference_id(range: crate::source::TextRange) -> String {
    format!("_bibliography_ref_{}", range.start().to_u32())
}

pub(super) fn render_bibliography_backrefs(
    output: &mut String,
    entry: &crate::catalog::BibliographyEntry,
) {
    for (index, reference) in entry.references.iter().enumerate() {
        BlockWriter::text(output, " ");
        let target = bibliography_reference_id(reference.range);
        let href = safe::SafeFragmentUrl::new(&target)
            .expect("generated bibliography reference IDs are control-free")
            .into_owned();
        BlockWriter::start(
            output,
            "a",
            &[
                classes(&["bibliography-backref"]),
                body::fragment_url("href", href),
            ],
        );
        BlockWriter::text(output, &format!("↩{}", index + 1));
        BlockWriter::end(output, "a");
    }
}

pub(super) fn render_heading(
    output: &mut String,
    heading: &Heading,
    id: &str,
    policy: &RenderPolicy,
    context: &mut InlineRenderContext<'_, '_>,
) {
    if !heading.well_formed {
        BlockWriter::start(output, "p", &[]);
        render_inlines(output, &heading.inlines, context);
        BlockWriter::end(output, "p");
        BlockWriter::line_break(output);
        return;
    }

    match heading.kind {
        HeadingKind::DocumentTitle if policy.render_document_title => {
            let roles = role_classes(&heading.metadata, context);
            BlockWriter::start(
                output,
                "h1",
                &[
                    body::classes_with_roles(&["document-title"], roles),
                    passive("id", id),
                ],
            );
            render_inlines(output, &heading.inlines, context);
            BlockWriter::end(output, "h1");
            BlockWriter::line_break(output);
        }
        HeadingKind::DocumentTitle => {}
        HeadingKind::Part => render_heading_level(output, heading, id, 1, context),
        HeadingKind::Section { level } | HeadingKind::Discrete { level } => {
            render_heading_level(output, heading, id, level, context);
        }
    }
}

pub(super) fn render_heading_level(
    output: &mut String,
    heading: &Heading,
    id: &str,
    level: u8,
    context: &mut InlineRenderContext<'_, '_>,
) {
    let name = match level {
        1 => "h1",
        2 => "h2",
        3 => "h3",
        4 => "h4",
        5 => "h5",
        _ => unreachable!("parser only produces supported heading levels"),
    };
    let appendix = context
        .structure
        .heading_at(heading.range)
        .is_some_and(|item| item.kind == crate::structure::SectionKind::Appendix);
    let fixed: &[&'static str] = if appendix { &["appendix"] } else { &[] };
    let roles = role_classes(&heading.metadata, context);
    let mut attributes = Vec::new();
    if !fixed.is_empty() || !roles.is_empty() {
        attributes.push(body::classes_with_roles(fixed, roles));
    }
    attributes.push(passive("id", id));
    BlockWriter::start(output, name, &attributes);
    if let Some(presentation) = context.presentation.heading_at(heading.range)
        && presentation.numbered
    {
        render_section_number(output, &presentation.number);
    }
    render_inlines(output, &heading.inlines, context);
    BlockWriter::end(output, name);
    BlockWriter::line_break(output);
}

pub(super) fn render_section_number(output: &mut String, number: &[u32]) {
    if number.is_empty() {
        return;
    }
    for (index, value) in number.iter().enumerate() {
        if index > 0 {
            BlockWriter::text(output, ".");
        }
        BlockWriter::text(output, &value.to_string());
    }
    BlockWriter::text(output, ". ");
}

pub(super) fn render_paragraph(
    output: &mut String,
    paragraph: &Paragraph,
    id: Option<&str>,
    context: &mut InlineRenderContext<'_, '_>,
) {
    // `lead` is the one role with a fixed class of its own; the policy decides
    // about every other role.
    let lead = paragraph
        .metadata
        .roles
        .iter()
        .any(|role| role.value == "lead")
        || paragraph
            .metadata
            .attributes
            .iter()
            .any(|attribute| attribute.name.is_none() && attribute.value == "lead");
    let fixed: &[&'static str] = if lead { &["lead"] } else { &[] };
    let attributes = block_attributes(id, &paragraph.metadata, fixed, context);
    BlockWriter::start(output, "p", &attributes);
    render_inlines(output, &paragraph.inlines, context);
    BlockWriter::end(output, "p");
    BlockWriter::line_break(output);
}

pub(super) fn render_inlines(
    output: &mut String,
    inlines: &[Inline],
    context: &mut InlineRenderContext<'_, '_>,
) {
    let plan = body::plan_inlines(inlines, context);
    body::serialize_inlines(output, &plan);
}

/// The `figcaption` of a verbatim block: the caption lead, then the title as
/// plain text, as the HTML contract promises for code samples.
fn render_verbatim_caption(
    output: &mut String,
    title: &crate::block_model::BlockTitle,
    range: crate::source::TextRange,
    context: &mut InlineRenderContext<'_, '_>,
) {
    BlockWriter::start(output, "figcaption", &[]);
    if let Some(lead) = caption_lead(range, context) {
        BlockWriter::text(output, &lead);
    }
    BlockWriter::text(
        output,
        &crate::projection::resolved_inline_text(&title.inlines),
    );
    BlockWriter::end(output, "figcaption");
    BlockWriter::line_break(output);
}

/// The `Figure 1. ` a numbered caption writes in front of its title.
fn caption_lead(
    range: crate::source::TextRange,
    context: &InlineRenderContext<'_, '_>,
) -> Option<String> {
    context
        .presentation
        .caption_at(range)
        .and_then(crate::caption::BlockCaption::lead)
}

/// An image block is a figure: the image, then the numbered caption built
/// from its block title. Without a title the figure has no caption.
pub(super) fn render_image_block(
    output: &mut String,
    paragraph: &Paragraph,
    explicit_id: Option<&str>,
    context: &mut InlineRenderContext<'_, '_>,
) {
    let attributes = block_attributes(explicit_id, &paragraph.metadata, &["image-block"], context);
    BlockWriter::start(output, "figure", &attributes);
    BlockWriter::line_break(output);
    render_inlines(output, &paragraph.inlines, context);
    BlockWriter::line_break(output);
    if let Some(title) = &paragraph.metadata.title {
        BlockWriter::start(output, "figcaption", &[]);
        if let Some(lead) = caption_lead(paragraph.range, context) {
            BlockWriter::text(output, &lead);
        }
        render_inlines(output, &title.inlines, context);
        BlockWriter::end(output, "figcaption");
        BlockWriter::line_break(output);
    }
    BlockWriter::end(output, "figure");
    BlockWriter::line_break(output);
}

pub(super) struct InlineRenderContext<'inputs, 'render> {
    pub(super) policy: &'inputs RenderPolicy,
    pub(super) input_usage: &'render mut RenderInputUsage<'inputs>,
    pub(super) diagnostics: &'render mut Vec<Diagnostic>,
    pub(super) catalogs: &'inputs crate::catalog::DocumentCatalogs,
    pub(super) identifiers: &'inputs crate::document::DocumentIdentifiers,
    pub(super) structure: &'inputs crate::structure::DocumentStructure,
    pub(super) presentation: &'inputs crate::presentation::DocumentPresentation,
    pub(super) generated_bibliography:
        Option<&'render generated_bibliography::PreparedGeneratedBibliography<'inputs>>,
}

pub(super) fn render_toc(
    output: &mut String,
    presentation: &crate::presentation::DocumentPresentation,
) {
    fn render_entries(
        output: &mut String,
        entries: &[crate::structure::TocEntry],
        presentation: &crate::presentation::DocumentPresentation,
    ) {
        if entries.is_empty() {
            return;
        }
        BlockWriter::start(output, "ul", &[]);
        BlockWriter::line_break(output);
        for entry in entries {
            BlockWriter::start(output, "li", &[]);
            let href = safe::SafeFragmentUrl::new(&entry.id)
                .expect("TOC identifiers are nonempty and control-free")
                .into_owned();
            BlockWriter::start(output, "a", &[body::fragment_url("href", href)]);
            if presentation
                .heading_at(entry.range)
                .is_some_and(|heading| heading.numbered)
            {
                render_section_number(output, &entry.number);
            }
            BlockWriter::text(output, &entry.title);
            BlockWriter::end(output, "a");
            render_entries(output, &entry.children, presentation);
            BlockWriter::end(output, "li");
            BlockWriter::line_break(output);
        }
        BlockWriter::end(output, "ul");
        BlockWriter::line_break(output);
    }

    if presentation.toc().is_empty() {
        return;
    }
    BlockWriter::start(output, "div", &[classes(&["toc"])]);
    BlockWriter::line_break(output);
    render_entries(output, presentation.toc(), presentation);
    BlockWriter::end(output, "div");
    BlockWriter::line_break(output);
}

pub(super) fn render_footnote_catalog(
    output: &mut String,
    catalogs: &crate::catalog::DocumentCatalogs,
    facts: &crate::resolved::DocumentFacts,
    context: &mut InlineRenderContext<'_, '_>,
) {
    if catalogs.footnotes().is_empty() {
        return;
    }
    BlockWriter::start(output, "div", &[classes(&["footnotes"])]);
    BlockWriter::line_break(output);
    BlockWriter::start(output, "ol", &[]);
    BlockWriter::line_break(output);
    for footnote in catalogs.footnotes() {
        let footnote_id = format!("_footnote_{}", footnote.number);
        BlockWriter::start(output, "li", &[passive("id", footnote_id)]);
        // The body is prose: links, formatting, and attribute references in
        // it render exactly as they do in a paragraph.
        match facts.footnote_body(footnote.definition_range) {
            Some(body) => {
                let plan = body::plan_inlines(body, context);
                body::serialize_inlines(output, &plan);
            }
            None => BlockWriter::inline_text(output, &footnote.text),
        }
        for (index, _) in footnote.occurrences.iter().enumerate() {
            BlockWriter::text(output, " ");
            let target = format!("_footnoteref_{}_{}", footnote.number, index + 1);
            let href = safe::SafeFragmentUrl::new(&target)
                .expect("generated footnote reference IDs are control-free")
                .into_owned();
            BlockWriter::start(
                output,
                "a",
                &[
                    classes(&["footnote-backref"]),
                    body::fragment_url("href", href),
                ],
            );
            BlockWriter::text(output, "↩");
            BlockWriter::end(output, "a");
        }
        BlockWriter::end(output, "li");
        BlockWriter::line_break(output);
    }
    BlockWriter::end(output, "ol");
    BlockWriter::line_break(output);
    BlockWriter::end(output, "div");
    BlockWriter::line_break(output);
}

pub(super) fn render_diagnostic(
    code: &str,
    message: &str,
    range: crate::source::TextRange,
) -> Diagnostic {
    Diagnostic {
        id: DiagnosticId::new(format!(
            "{code}@{}:{}",
            range.start().to_u32(),
            range.end().to_u32()
        )),
        code: DiagnosticCode::new(code),
        severity: Severity::Warning,
        message: message.to_owned(),
        range,
        related: Vec::new(),
        fixes: Vec::new(),
    }
}

pub(super) fn append_plan_diagnostics(
    diagnostics: &mut Vec<Diagnostic>,
    planned: impl IntoIterator<Item = plan::PlanDiagnostic>,
) {
    diagnostics.extend(planned.into_iter().map(|diagnostic| {
        render_diagnostic(diagnostic.code, diagnostic.message, diagnostic.range)
    }));
}

pub(super) fn render_input_diagnostic(
    code: &str,
    domain: &str,
    message: &str,
    range: crate::source::TextRange,
) -> Diagnostic {
    let mut diagnostic = render_diagnostic(code, message, range);
    diagnostic.id = DiagnosticId::new(format!(
        "{code}:{domain}@{}:{}",
        range.start().to_u32(),
        range.end().to_u32()
    ));
    diagnostic
}

pub(super) fn render_unsupported(output: &mut String, unsupported: &Unsupported, id: Option<&str>) {
    BlockWriter::start(output, "p", &optional_id(id));
    BlockWriter::text(output, &unsupported.raw);
    BlockWriter::end(output, "p");
    BlockWriter::line_break(output);
}

pub(super) fn optional_id(id: Option<&str>) -> Vec<body::PlannedAttribute> {
    id.map(|id| vec![passive("id", id)]).unwrap_or_default()
}

/// `id` and `class` for a block element: the explicit anchor, the renderer's
/// fixed classes, and the block roles the render policy admits as `role-<name>`.
///
/// Every block element goes through here so a role behaves the same on a
/// paragraph, a wrapper, a table, or a list, and so a block never carries two
/// `class` attributes.
pub(super) fn block_attributes(
    explicit_id: Option<&str>,
    metadata: &crate::block_model::BlockMetadata,
    fixed_classes: &[&'static str],
    context: &mut InlineRenderContext<'_, '_>,
) -> Vec<body::PlannedAttribute> {
    let mut attributes = optional_id(explicit_id);
    let roles = role_classes(metadata, context);
    if !fixed_classes.is_empty() || !roles.is_empty() {
        attributes.push(body::classes_with_roles(fixed_classes, roles));
    }
    attributes
}

/// The role classes the policy admits, in authored order without repeats. A
/// role the host does not list is dropped, with a diagnostic when asked for.
fn role_classes(
    metadata: &crate::block_model::BlockMetadata,
    context: &mut InlineRenderContext<'_, '_>,
) -> Vec<super::safe::RoleClass> {
    let mut classes = Vec::new();
    for (role, range) in metadata.role_names() {
        if context.policy.roles.allows(role) {
            if let Some(class) = super::safe::RoleClass::new(role)
                && !classes.contains(&class)
            {
                classes.push(class);
            }
        } else if context.policy.roles.unknown == UnknownRole::Diagnostic {
            context.diagnostics.push(render_diagnostic(
                "role-not-allowed",
                "block role is not allowed by the render policy",
                range,
            ));
        }
    }
    classes
}
