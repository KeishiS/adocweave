//! Numbered captions for figures, tables, and examples.
//!
//! A block title on an image block, a table, or an example block is a caption:
//! the language numbers it in reading order and prefixes the number with the
//! family's caption word (`Figure`, `Table`, `Example`). The word comes from the
//! `figure-caption`, `table-caption`, and `example-caption` document attributes
//! at the block's position, so a document can translate it (`:figure-caption: 図`)
//! or switch numbering off (`:figure-caption!:`). Cross references use the
//! numbered label (`Figure 1`) as their display text.

use std::collections::BTreeMap;

use crate::attributes::AttributeEnvironment;
use crate::block_model::{AstBlock, AstDocument, BlockMetadata, Paragraph};
use crate::inline_model::{Inline, MacroForm, StandardMacro, StandardMacroKind};
use crate::presentation::{BlockId, DocumentIndex};
use crate::source::TextRange;

/// The kind of block a numbered caption belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptionFamily {
    Figure,
    Table,
    Example,
}

impl CaptionFamily {
    /// The document attribute that holds the caption word for this family.
    pub const fn attribute_name(self) -> &'static str {
        match self {
            Self::Figure => "figure-caption",
            Self::Table => "table-caption",
            Self::Example => "example-caption",
        }
    }

    /// The caption word the language uses when the attribute is never set.
    pub const fn default_prefix(self) -> &'static str {
        match self {
            Self::Figure => "Figure",
            Self::Table => "Table",
            Self::Example => "Example",
        }
    }
}

/// A titled block that carries a numbered caption.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockCaption {
    pub block: BlockId,
    pub range: TextRange,
    pub family: CaptionFamily,
    /// Position within the family, counting from 1 in reading order. Only
    /// captions with a prefix are numbered.
    pub number: Option<u32>,
    /// The caption word in effect at the block, or `None` when the document
    /// unset the family's caption attribute.
    pub prefix: Option<String>,
    /// The block title as plain text.
    pub title: String,
}

impl BlockCaption {
    /// The short label a cross reference shows, such as `Figure 1`.
    pub fn label(&self) -> Option<String> {
        match (&self.prefix, self.number) {
            (Some(prefix), Some(number)) => Some(format!("{prefix} {number}")),
            _ => None,
        }
    }

    /// The text written in front of the title, such as `Figure 1. `, or
    /// nothing when the caption is not numbered.
    pub fn lead(&self) -> Option<String> {
        self.label().map(|label| format!("{label}. "))
    }

    /// The complete caption text: the lead followed by the title.
    pub fn text(&self) -> String {
        match self.lead() {
            Some(lead) => format!("{lead}{}", self.title),
            None => self.title.clone(),
        }
    }
}

/// Captions of one document, addressable by the block's source range.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CaptionIndex {
    captions: Vec<BlockCaption>,
    ordinals: BTreeMap<TextRange, usize>,
}

impl CaptionIndex {
    pub(crate) fn captions(&self) -> &[BlockCaption] {
        &self.captions
    }

    pub(crate) fn caption_at(&self, range: TextRange) -> Option<&BlockCaption> {
        self.ordinals
            .get(&range)
            .and_then(|ordinal| self.captions.get(*ordinal))
    }
}

/// The block image macro a paragraph consists of, if the paragraph is one.
///
/// `image::target[]` on a line of its own is an image block: the paragraph
/// holds exactly that macro and nothing else.
pub(crate) fn block_image(paragraph: &Paragraph) -> Option<&StandardMacro> {
    if paragraph.admonition.is_some() {
        return None;
    }
    match paragraph.inlines.as_slice() {
        [Inline::Macro(node)]
            if node.kind == StandardMacroKind::Image && node.form == MacroForm::Block =>
        {
            Some(node)
        }
        _ => None,
    }
}

/// The caption family of a block, if blocks of its kind carry captions.
fn caption_family(block: &AstBlock) -> Option<CaptionFamily> {
    match block {
        AstBlock::Paragraph(paragraph) => block_image(paragraph).map(|_| CaptionFamily::Figure),
        AstBlock::Delimited(delimited) => match (&delimited.content, &delimited.presentation) {
            (crate::block_model::DelimitedContent::Table(_), _) => Some(CaptionFamily::Table),
            (crate::block_model::DelimitedContent::Compound(_), None)
                if delimited.kind == crate::block_model::DelimitedBlockKind::Example =>
            {
                Some(CaptionFamily::Example)
            }
            _ => None,
        },
        _ => None,
    }
}

fn titled(metadata: &BlockMetadata) -> Option<String> {
    metadata
        .title
        .as_ref()
        .map(|title| crate::projection::resolved_inline_text(&title.inlines))
}

/// Numbers every titled figure, table, and example in reading order.
///
/// The caption word is resolved where the block starts, so an attribute entry
/// in the middle of the document changes the blocks after it and no others.
pub(crate) fn build_captions(
    document: &AstDocument,
    index: &DocumentIndex,
    attributes: &AttributeEnvironment,
    checkpoint: &mut crate::cancellation::CancellationCheckpoint<'_>,
) -> Result<CaptionIndex, ()> {
    let mut captions = CaptionIndex::default();
    let mut counters: BTreeMap<&'static str, u32> = BTreeMap::new();
    let walked = crate::walker::try_walk_ast(document, |node| {
        if checkpoint.is_cancelled() {
            return std::ops::ControlFlow::Break(());
        }
        let crate::walker::SemanticNode::Block(block) = node else {
            return std::ops::ControlFlow::Continue(());
        };
        let Some(family) = caption_family(block) else {
            return std::ops::ControlFlow::Continue(());
        };
        let Some(title) = titled(block.metadata()) else {
            return std::ops::ControlFlow::Continue(());
        };
        let range = block.range();
        let Some(block_id) = index.block_id_at(range) else {
            return std::ops::ControlFlow::Continue(());
        };
        // `None`: never set, the language's word applies. `Ok(None)`: unset by
        // the document, the caption is just the title.
        let prefix = match attributes.resolve_at(family.attribute_name(), range.start()) {
            None => Some(family.default_prefix().to_owned()),
            Some(resolved) => resolved
                .value
                .ok()
                .flatten()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
        };
        let number = prefix.as_ref().map(|_| {
            let counter = counters.entry(family.attribute_name()).or_insert(0);
            *counter += 1;
            *counter
        });
        let ordinal = captions.captions.len();
        captions.captions.push(BlockCaption {
            block: block_id,
            range,
            family,
            number,
            prefix,
            title,
        });
        captions.ordinals.insert(range, ordinal);
        std::ops::ControlFlow::Continue(())
    });
    if walked.is_break() {
        return Err(());
    }
    Ok(captions)
}
