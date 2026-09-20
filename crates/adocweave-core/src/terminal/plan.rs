//! The order in which document material reaches the page.

use crate::block_model::{AstBlock, AstDocument, Heading, HeadingKind};
use crate::presentation::{GeneratedLayoutNode, LayoutNode, LayoutScope};

use super::TerminalPolicy;

/// One item of the rendered body.
pub(super) enum BodyStep<'document> {
    Block {
        block: &'document AstBlock,
        /// Whether the author and revision lines follow this block. They follow
        /// the document title and nothing else.
        header_metadata: bool,
    },
    TableOfContents,
    FootnoteCatalog,
}

/// Walks the backend-independent layout, so generated material lands where the
/// document says it does rather than where this backend happens to walk.
pub(super) fn body_steps<'document>(
    document: &'document AstDocument,
    policy: &TerminalPolicy,
) -> Vec<BodyStep<'document>> {
    let mut steps = Vec::new();
    append(document, document.layout().nodes(), policy, &mut steps);
    steps
}

fn append<'document>(
    document: &'document AstDocument,
    nodes: &[LayoutNode],
    policy: &TerminalPolicy,
    steps: &mut Vec<BodyStep<'document>>,
) {
    for node in nodes {
        match node {
            LayoutNode::Generated(GeneratedLayoutNode::TableOfContents) => {
                steps.push(BodyStep::TableOfContents);
            }
            LayoutNode::Generated(GeneratedLayoutNode::FootnoteCatalog) => {
                steps.push(BodyStep::FootnoteCatalog);
            }
            LayoutNode::Section {
                scope: LayoutScope::Bibliography,
                nodes,
            } => append(document, nodes, policy, steps),
            LayoutNode::Block(block_id) => {
                let block = document
                    .top_level_block(*block_id)
                    .expect("layout only contains top-level blocks");
                steps.push(BodyStep::Block {
                    block,
                    header_metadata: policy.render_document_title
                        && matches!(
                            block,
                            AstBlock::Heading(Heading {
                                kind: HeadingKind::DocumentTitle,
                                ..
                            })
                        ),
                });
            }
        }
    }
}
