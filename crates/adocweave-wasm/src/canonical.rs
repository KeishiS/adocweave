//! Canonical string products exposed by the WebAssembly protocol.

use std::fmt::Write as _;

use adocweave_core::Analysis;
use adocweave_core::resolution::ReferenceKey;
use adocweave_core::semantic::{Block, BlockMetadata, Inline, ListBlock, ListItem};
use adocweave_core::text::TextRange;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalAst {
    schema_version: u16,
    blocks: Vec<CanonicalNode>,
    attributes: Vec<CanonicalNode>,
    anchors: Vec<CanonicalNode>,
}

#[derive(serde::Serialize)]
struct CanonicalNode {
    kind: &'static str,
    range: [u32; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<CanonicalNode>,
}

fn canonical_ast_document(document: &adocweave_core::semantic::Document) -> String {
    let dto = CanonicalAst {
        schema_version: 2,
        blocks: document.blocks().iter().map(block_node).collect(),
        attributes: document
            .attribute_occurrences()
            .iter()
            .map(|attribute| CanonicalNode {
                kind: "attribute",
                range: range(attribute.range),
                value: Some(format!(
                    "{}={}",
                    attribute.name, attribute.value.folded_text
                )),
                children: Vec::new(),
            })
            .collect(),
        anchors: document
            .anchors()
            .iter()
            .map(|anchor| CanonicalNode {
                kind: "anchor",
                range: range(anchor.range),
                value: Some(anchor.id.clone()),
                children: Vec::new(),
            })
            .collect(),
    };
    serde_json::to_string(&dto).expect("canonical DTO contains owned serializable values")
}

/// Returns the stable canonical AST JSON for one completed analysis.
///
/// This focused serializer is also used by production adapters whose public
/// protocol intentionally exposes the canonical AST as a JSON string.
pub(crate) fn canonical_ast(analysis: &Analysis) -> String {
    canonical_ast_document(analysis.document())
}

fn block_node(block: &Block) -> CanonicalNode {
    let mut node = match block {
        Block::Heading(node) => CanonicalNode {
            kind: match node.kind {
                adocweave_core::semantic::HeadingKind::DocumentTitle => "document-title",
                adocweave_core::semantic::HeadingKind::Part => "part",
                adocweave_core::semantic::HeadingKind::Section { .. } => "section",
                adocweave_core::semantic::HeadingKind::Discrete { .. } => "discrete-heading",
            },
            range: range(node.range),
            value: Some(node.text.clone()),
            children: inline_nodes(&node.inlines),
        },
        Block::Paragraph(node) => CanonicalNode {
            kind: "paragraph",
            range: range(node.range),
            value: Some(node.value.clone()),
            children: node
                .admonition
                .iter()
                .map(|presentation| CanonicalNode {
                    kind: "admonition-presentation",
                    range: range(presentation.label_range),
                    value: Some(presentation.kind.label().to_owned()),
                    children: Vec::new(),
                })
                .chain(inline_nodes(&node.inlines))
                .collect(),
        },
        Block::LiteralParagraph(node) => leaf("literal-paragraph", node.range, &node.value),
        Block::Break(node) => CanonicalNode {
            kind: match node.kind {
                adocweave_core::semantic::BreakKind::Thematic => "thematic-break",
                adocweave_core::semantic::BreakKind::Page => "page-break",
            },
            range: range(node.range),
            value: None,
            children: Vec::new(),
        },
        Block::Verbatim(node) => CanonicalNode {
            kind: match node.kind {
                adocweave_core::semantic::VerbatimKind::Listing => "listing-block",
                adocweave_core::semantic::VerbatimKind::Literal => "literal-block",
                adocweave_core::semantic::VerbatimKind::Source(_) => "source-block",
            },
            range: range(node.range),
            value: Some(match &node.kind {
                adocweave_core::semantic::VerbatimKind::Source(source) => format!(
                    "{}:{}",
                    source.language.as_deref().unwrap_or(""),
                    node.value
                ),
                adocweave_core::semantic::VerbatimKind::Listing
                | adocweave_core::semantic::VerbatimKind::Literal => node.value.clone(),
            }),
            children: Vec::new(),
        },
        Block::List(node) => list_node(node),
        Block::Math(node) => leaf("math-block", node.range, &node.value),
        Block::Delimited(node) => {
            let (value, mut children) = match &node.content {
                adocweave_core::semantic::DelimitedContent::Compound(children) => (
                    Some(node.delimiter.clone()),
                    children.iter().map(block_node).collect(),
                ),
                adocweave_core::semantic::DelimitedContent::Verbatim(value)
                | adocweave_core::semantic::DelimitedContent::Passthrough(value) => {
                    (Some(value.clone()), Vec::new())
                }
                adocweave_core::semantic::DelimitedContent::Table(table) => (
                    Some(format!("{:?}", table.format).to_ascii_lowercase()),
                    std::iter::once(CanonicalNode {
                        kind: "table-presentation",
                        range: range(node.range),
                        value: Some(table_presentation_value(&table.presentation)),
                        children: Vec::new(),
                    })
                    .chain(table.rows.iter().map(|row| {
                        CanonicalNode {
                            kind: "table-row",
                            range: range(row.range),
                            value: Some(format!("{:?}", row.section).to_ascii_lowercase()),
                            children: row
                                .cells
                                .iter()
                                .map(|cell| {
                                    let children = match &cell.content {
                                        adocweave_core::semantic::TableCellContent::Inlines(
                                            inlines,
                                        ) => inline_nodes(inlines),
                                        adocweave_core::semantic::TableCellContent::AsciiDoc(
                                            blocks,
                                        ) => blocks.iter().map(block_node).collect(),
                                        adocweave_core::semantic::TableCellContent::Verbatim(_) => {
                                            Vec::new()
                                        }
                                    };
                                    CanonicalNode {
                                        kind: "table-cell",
                                        range: range(cell.range),
                                        value: Some(cell.raw.clone()),
                                        children,
                                    }
                                })
                                .collect(),
                        }
                    }))
                    .collect(),
                ),
            };
            if let Some(presentation) = &node.presentation {
                children.insert(
                    0,
                    CanonicalNode {
                        kind: match presentation {
                            adocweave_core::semantic::DelimitedPresentation::Admonition(_) => {
                                "admonition-presentation"
                            }
                            adocweave_core::semantic::DelimitedPresentation::Quote(_) => {
                                "quote-presentation"
                            }
                            adocweave_core::semantic::DelimitedPresentation::Collapsible(_) => {
                                "collapsible-presentation"
                            }
                        },
                        range: range(node.range),
                        value: Some(match presentation {
                            adocweave_core::semantic::DelimitedPresentation::Admonition(value) => {
                                value.kind.label().to_owned()
                            }
                            adocweave_core::semantic::DelimitedPresentation::Collapsible(value) => {
                                if value.open { "open" } else { "closed" }.to_owned()
                            }
                            adocweave_core::semantic::DelimitedPresentation::Quote(value) => {
                                format!(
                                    "{}:{}:{}",
                                    match value.kind {
                                        adocweave_core::semantic::QuoteKind::Quote => "quote",
                                        adocweave_core::semantic::QuoteKind::Verse => "verse",
                                    },
                                    value.attribution.as_ref().map_or("", |value| &value.value),
                                    value.citation.as_ref().map_or("", |value| &value.value),
                                )
                            }
                        }),
                        children: Vec::new(),
                    },
                );
            }
            CanonicalNode {
                kind: match node.kind {
                    adocweave_core::semantic::DelimitedBlockKind::Comment => "comment-block",
                    adocweave_core::semantic::DelimitedBlockKind::Example => "example-block",
                    adocweave_core::semantic::DelimitedBlockKind::Listing => "listing-block",
                    adocweave_core::semantic::DelimitedBlockKind::Literal => "literal-block",
                    adocweave_core::semantic::DelimitedBlockKind::Open => "open-block",
                    adocweave_core::semantic::DelimitedBlockKind::Sidebar => "sidebar-block",
                    adocweave_core::semantic::DelimitedBlockKind::Pass => "pass-block",
                    adocweave_core::semantic::DelimitedBlockKind::Quote => "quote-block",
                    adocweave_core::semantic::DelimitedBlockKind::Table => "table-block",
                },
                range: range(node.range),
                value,
                children,
            }
        }
        Block::Unsupported(node) => leaf("unsupported", node.range, &node.raw),
    };
    let mut children = metadata_nodes(block.metadata());
    children.append(&mut node.children);
    node.children = children;
    node
}

fn table_presentation_value(presentation: &adocweave_core::semantic::TablePresentation) -> String {
    format!(
        "frame={};grid={};stripes={};width={};autowidth={}",
        match presentation.frame {
            adocweave_core::semantic::TableFrame::All => "all",
            adocweave_core::semantic::TableFrame::Ends => "ends",
            adocweave_core::semantic::TableFrame::None => "none",
            adocweave_core::semantic::TableFrame::Sides => "sides",
        },
        match presentation.grid {
            adocweave_core::semantic::TableGrid::All => "all",
            adocweave_core::semantic::TableGrid::Columns => "cols",
            adocweave_core::semantic::TableGrid::None => "none",
            adocweave_core::semantic::TableGrid::Rows => "rows",
        },
        match presentation.stripes {
            adocweave_core::semantic::TableStripes::All => "all",
            adocweave_core::semantic::TableStripes::Even => "even",
            adocweave_core::semantic::TableStripes::Hover => "hover",
            adocweave_core::semantic::TableStripes::None => "none",
            adocweave_core::semantic::TableStripes::Odd => "odd",
        },
        presentation
            .width
            .map_or_else(|| "none".to_owned(), |width| format!("{width}%")),
        presentation.autowidth,
    )
}

fn metadata_nodes(metadata: &BlockMetadata) -> Vec<CanonicalNode> {
    let mut nodes = Vec::new();
    if let Some(title) = &metadata.title {
        nodes.push(CanonicalNode {
            kind: "block-title",
            range: range(title.range),
            value: Some(title.value.clone()),
            children: inline_nodes(&title.inlines),
        });
    }
    if let Some(id) = &metadata.id {
        nodes.push(leaf("block-id", id.range, &id.value));
    }
    nodes.extend(
        metadata
            .roles
            .iter()
            .map(|role| leaf("block-role", role.range, &role.value)),
    );
    nodes.extend(
        metadata
            .options
            .iter()
            .map(|option| leaf("block-option", option.range, &option.value)),
    );
    nodes.extend(metadata.attributes.iter().map(|attribute| CanonicalNode {
        kind: "element-attribute",
        range: range(attribute.range),
        value: Some(attribute.name.as_ref().map_or_else(
            || attribute.value.clone(),
            |name| format!("{name}={}", attribute.value),
        )),
        children: Vec::new(),
    }));
    nodes
}

fn list_node(list: &ListBlock) -> CanonicalNode {
    CanonicalNode {
        kind: match list.kind {
            adocweave_core::semantic::ListKind::Unordered => "unordered-list",
            adocweave_core::semantic::ListKind::Ordered => "ordered-list",
            adocweave_core::semantic::ListKind::Description => "description-list",
            adocweave_core::semantic::ListKind::Callout => "callout-list",
        },
        range: range(list.range),
        value: None,
        children: list.items.iter().map(list_item_node).collect(),
    }
}

fn list_item_node(item: &ListItem) -> CanonicalNode {
    let mut children = item
        .terms
        .iter()
        .map(|term| CanonicalNode {
            kind: "description-term",
            range: range(term.range),
            value: Some(term.text.clone()),
            children: inline_nodes(&term.inlines),
        })
        .collect::<Vec<_>>();
    children.extend(inline_nodes(&item.inlines));
    children.extend(item.children.iter().map(list_node));
    children.extend(item.continuations.iter().map(block_node));
    CanonicalNode {
        kind: "list-item",
        range: range(item.range),
        value: Some(match (item.checklist, item.callout_id) {
            (Some(adocweave_core::semantic::ChecklistState::Checked), _) => {
                format!("checked:{}", item.text)
            }
            (Some(adocweave_core::semantic::ChecklistState::Unchecked), _) => {
                format!("unchecked:{}", item.text)
            }
            (_, Some(id)) => format!("callout-{id}:{}", item.text),
            _ => item.text.clone(),
        }),
        children,
    }
}

fn inline_nodes(inlines: &[Inline]) -> Vec<CanonicalNode> {
    inlines.iter().map(inline_node).collect()
}

fn inline_node(inline: &Inline) -> CanonicalNode {
    match inline {
        Inline::Text(node) => leaf("text", node.range, &node.value),
        Inline::Literal {
            range: node_range,
            value,
            ..
        } => leaf("monospace", *node_range, value),
        Inline::Styled {
            style,
            range: node_range,
            children,
            ..
        } => CanonicalNode {
            kind: match style {
                adocweave_core::semantic::InlineStyle::Strong => "strong",
                adocweave_core::semantic::InlineStyle::Emphasis => "emphasis",
                adocweave_core::semantic::InlineStyle::Highlight => "highlight",
                adocweave_core::semantic::InlineStyle::Subscript => "subscript",
                adocweave_core::semantic::InlineStyle::Superscript => "superscript",
                adocweave_core::semantic::InlineStyle::CurvedDoubleQuote => "curved-double-quote",
                adocweave_core::semantic::InlineStyle::CurvedSingleQuote => "curved-single-quote",
            },
            range: range(*node_range),
            value: None,
            children: inline_nodes(children),
        },
        Inline::AttributeReference {
            range: node_range,
            name,
            ..
        } => leaf("attribute-reference", *node_range, name),
        Inline::Link(node) => CanonicalNode {
            kind: "link",
            range: range(node.range),
            value: Some(node.target.clone()),
            children: inline_nodes(&node.label),
        },
        Inline::Reference(node) => CanonicalNode {
            kind: match node.target {
                Some(ReferenceKey::Local { .. }) => "local-reference",
                Some(ReferenceKey::Document { .. }) => "document-reference",
                Some(ReferenceKey::Scheme { .. }) => "scheme-reference",
                None => "invalid-reference",
            },
            range: range(node.range),
            value: Some(node.target_source.clone()),
            children: inline_nodes(&node.label),
        },
        Inline::Formula(node) => leaf("inline-math", node.range, &node.value),
        Inline::Macro(node) => CanonicalNode {
            kind: "standard-macro",
            range: range(node.range),
            value: Some(format!("{:?}:{}", node.kind, node.target)),
            children: Vec::new(),
        },
        Inline::Passthrough { range, value, .. } => leaf("passthrough", *range, value),
        Inline::HardBreak { range: node_range } => CanonicalNode {
            kind: "hard-break",
            range: range(*node_range),
            value: None,
            children: Vec::new(),
        },
    }
}

fn leaf(kind: &'static str, node_range: TextRange, value: &str) -> CanonicalNode {
    CanonicalNode {
        kind,
        range: range(node_range),
        value: Some(value.to_owned()),
        children: Vec::new(),
    }
}

fn range(value: TextRange) -> [u32; 2] {
    [value.start().to_u32(), value.end().to_u32()]
}

pub(crate) fn canonical_syntax(analysis: &Analysis) -> String {
    let mut output = analysis.syntax().snapshot();
    output.push_str("Tokens\n");
    for token in analysis.syntax().tokens() {
        writeln!(
            output,
            "  {:?}@{}..{}",
            token.kind,
            token.range.start().to_u32(),
            token.range.end().to_u32()
        )
        .expect("writing to a String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use adocweave_core::{AnalysisOptions, Engine};

    use super::*;

    #[test]
    fn canonical_products_are_deterministic() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze("= Title\n\n[[target]]\n== Section\n\n<<target,Here>>\n")
            .expect("analysis");

        assert_eq!(canonical_syntax(&analysis), canonical_syntax(&analysis));
        assert_eq!(canonical_ast(&analysis), canonical_ast(&analysis));
        assert!(canonical_syntax(&analysis).contains("Document@"));
        assert!(canonical_ast(&analysis).contains("local-reference"));
    }

    #[test]
    fn canonical_ast_exposes_backend_neutral_block_metadata() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze(".Title\n[#item.role%collapsible,kind=demo]\nText\n")
            .expect("analysis");
        let value: serde_json::Value =
            serde_json::from_str(&canonical_ast(&analysis)).expect("canonical JSON");
        let children = value["blocks"][0]["children"].as_array().expect("children");
        assert_eq!(value["schemaVersion"], 2);
        assert_eq!(children[0]["kind"], "block-title");
        assert_eq!(children[1]["kind"], "block-id");
        assert_eq!(children[2]["kind"], "block-role");
        assert_eq!(children[3]["kind"], "block-option");
        assert_eq!(children[4]["value"], "kind=demo");
    }

    #[test]
    fn canonical_ast_distinguishes_delimited_content_models() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze("====\ninside\n====\n\n++++\n<tag>\n++++\n")
            .expect("analysis");
        let value: serde_json::Value =
            serde_json::from_str(&canonical_ast(&analysis)).expect("canonical JSON");
        assert_eq!(value["blocks"][0]["kind"], "example-block");
        assert_eq!(value["blocks"][0]["children"][0]["kind"], "paragraph");
        assert_eq!(value["blocks"][1]["kind"], "pass-block");
        assert_eq!(value["blocks"][1]["value"], "<tag>\n");
    }

    #[test]
    fn canonical_ast_exposes_typed_table_presentation() {
        let analysis = Engine::new(AnalysisOptions::default())
            .analyze(".Caption\n[frame=ends,grid=rows,stripes=odd,width=75%]\n|===\n|cell\n|===\n")
            .expect("analysis");
        let value: serde_json::Value =
            serde_json::from_str(&canonical_ast(&analysis)).expect("canonical JSON");
        assert_eq!(value["blocks"][0]["kind"], "table-block");
        let presentation = value["blocks"][0]["children"]
            .as_array()
            .expect("table children")
            .iter()
            .find(|child| child["kind"] == "table-presentation")
            .expect("table presentation");
        assert_eq!(
            presentation["value"],
            "frame=ends;grid=rows;stripes=odd;width=75%;autowidth=false"
        );
    }
}
