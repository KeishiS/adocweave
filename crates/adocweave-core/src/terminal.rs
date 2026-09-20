//! Terminal output backend.
//!
//! The backend lays a document out for a fixed number of columns and returns
//! lines built from styled spans. It never writes escape sequences. A host
//! decides how to encode [`TerminalStyle`], because only the host knows whether
//! its output is a terminal, what the reader set `NO_COLOR` to, and which
//! sequences that terminal understands. Keeping the decision there is also what
//! lets a non-terminal host, such as a browser page, use the same layout.
//!
//! The layout is readable without any styling at all. Depth, separation, and
//! labels are carried by indentation, rules, and text, so a host that drops
//! every style still produces a document a reader can follow.

mod blocks;
mod inline;
mod layout;
mod numbering;
mod plan;
mod table;

#[cfg(test)]
mod tests;

use crate::block_model::AdmonitionKind;
use crate::diagnostic::Diagnostic;

/// How wide the rendered lines may be.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalWidth {
    /// Wrap to this many columns.
    Columns(u16),
    /// Do not wrap. A host that reflows the text itself asks for this.
    Unlimited,
}

impl Default for TerminalWidth {
    fn default() -> Self {
        Self::Columns(80)
    }
}

/// How wide the characters of ambiguous East Asian width are drawn.
///
/// Characters such as `…`, `→`, and `■` occupy one column in most terminals and
/// two in a terminal configured for East Asian text. The two settings disagree
/// only about those characters; everything else is measured the same way.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AmbiguousWidth {
    #[default]
    Narrow,
    Wide,
}

/// Columns placed in front of nested material.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IndentPolicy {
    /// Columns added for each heading level below the outermost section level.
    pub heading_step: u16,
    /// Columns in front of content that sits inside another block, such as the
    /// description of a term or the body of a collapsible block.
    pub nested: u16,
}

impl Default for IndentPolicy {
    fn default() -> Self {
        Self {
            heading_step: 2,
            nested: 2,
        }
    }
}

/// The symbols a list item is written with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListMarkers {
    /// One marker per level of nesting, used in turn and then from the start
    /// again for deeper levels.
    pub unordered: [char; 3],
    /// What follows the number of an ordered item.
    pub ordered_separator: char,
    /// The checked and unchecked boxes of a checklist. They are written in
    /// ASCII because the box characters are missing from many terminal fonts.
    pub checklist: [&'static str; 2],
}

impl Default for ListMarkers {
    fn default() -> Self {
        Self {
            unordered: ['•', '◦', '▪'],
            ordered_separator: '.',
            checklist: ["[ ]", "[x]"],
        }
    }
}

/// How a table is drawn.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TableBorders {
    /// Box-drawing characters, which every modern terminal can show.
    #[default]
    Unicode,
    /// Only ASCII, for a terminal or a font that cannot draw boxes.
    Ascii,
    /// No lines at all. Columns are separated by spacing.
    None,
}

/// Backend settings that a host chooses once for a rendered document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalPolicy {
    pub width: TerminalWidth,
    pub ambiguous_width: AmbiguousWidth,
    pub render_document_title: bool,
    pub indent: IndentPolicy,
    pub list_markers: ListMarkers,
    pub table_borders: TableBorders,
}

impl Default for TerminalPolicy {
    fn default() -> Self {
        Self {
            width: TerminalWidth::default(),
            ambiguous_width: AmbiguousWidth::default(),
            render_document_title: true,
            indent: IndentPolicy::default(),
            list_markers: ListMarkers::default(),
            table_borders: TableBorders::default(),
        }
    }
}

/// What a span of text is, rather than which color it gets.
///
/// A host maps a role to whatever its output can show. Naming the purpose and
/// not the appearance is what lets one host use color, another bold text, and a
/// third nothing at all, without the backend knowing which.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TerminalRole {
    /// Ordinary body text, and the indentation in front of it.
    #[default]
    Text,
    DocumentTitle,
    Heading {
        level: u8,
    },
    /// Authors, revision, and similar facts taken from the document header.
    Metadata,
    /// A list marker, a callout number, or another symbol that stands in front
    /// of content.
    Marker,
    /// A horizontal rule or a block border.
    Rule,
    /// A block title or a generated caption such as `Figure 1.`.
    Caption,
    /// Text that stays readable but should recede, such as a URL written after
    /// the text that links to it.
    Muted,
    /// Inline monospace text.
    Monospace,
    /// A verbatim, listing, or source block.
    Code,
    Math,
    Link,
    Reference,
    UnresolvedReference,
    Admonition(AdmonitionKind),
    Quote,
    Attribution,
    TableHeader,
    TableBorder,
    FootnoteMarker,
    FootnoteText,
    /// A placeholder for an image, video, or audio resource the terminal cannot
    /// display.
    MediaPlaceholder,
    /// Text kept from a construct the parser did not understand.
    Unsupported,
}

/// A role together with the emphasis applied on top of it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TerminalStyle {
    pub role: TerminalRole,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub dim: bool,
    /// Foreground and background exchanged, used for highlighted text.
    pub inverse: bool,
}

impl TerminalStyle {
    /// A style that carries `role` and no emphasis.
    pub const fn of(role: TerminalRole) -> Self {
        Self {
            role,
            bold: false,
            italic: false,
            underline: false,
            dim: false,
            inverse: false,
        }
    }
}

/// A run of text that shares one style.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalSpan {
    pub text: String,
    pub style: TerminalStyle,
    /// The URL this run points at, when it has one. A host that can write
    /// terminal hyperlinks uses it; every other host ignores it, because the
    /// rendered text already says where the link goes.
    pub link: Option<String>,
}

impl TerminalSpan {
    /// A span of `text` carrying `style` and no link.
    pub fn new(text: impl Into<String>, style: TerminalStyle) -> Self {
        Self {
            text: text.into(),
            style,
            link: None,
        }
    }
}

/// One rendered line. Its text never contains a line break.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalLine {
    pub spans: Vec<TerminalSpan>,
}

impl TerminalLine {
    /// The line without any styling.
    pub fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }

    /// How many columns the line occupies.
    pub fn display_width(&self, ambiguous: AmbiguousWidth) -> usize {
        self.spans
            .iter()
            .map(|span| display_width(&span.text, ambiguous))
            .sum()
    }

    /// Whether the line has no text at all.
    pub fn is_empty(&self) -> bool {
        self.spans.iter().all(|span| span.text.is_empty())
    }
}

/// The laid-out document.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalDocument {
    pub lines: Vec<TerminalLine>,
}

impl TerminalDocument {
    /// The document without any styling, one line per rendered line. A
    /// non-empty document ends with a line break.
    pub fn to_plain_text(&self) -> String {
        let mut text = String::new();
        for line in &self.lines {
            text.push_str(&line.text());
            text.push('\n');
        }
        text
    }
}

/// The result of rendering a document for a terminal.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalOutput {
    pub package_version: &'static str,
    pub document: TerminalDocument,
    pub diagnostics: Vec<Diagnostic>,
}

/// How many columns `text` occupies in a terminal.
///
/// Combining marks and other zero-width characters add nothing, and a character
/// drawn on two columns counts as two. Control characters are counted as
/// nothing, because they are removed before a line is built.
pub fn display_width(text: &str, ambiguous: AmbiguousWidth) -> usize {
    text.chars()
        .map(|character| layout::character_width(character, ambiguous))
        .sum()
}

/// Lays `document` out for a terminal.
pub fn render(document: &crate::document::Document, policy: &TerminalPolicy) -> TerminalOutput {
    let mut diagnostics = Vec::new();
    let lines = blocks::render_document(document.inner(), policy);
    crate::diagnostic::sort_diagnostics(&mut diagnostics);
    TerminalOutput {
        package_version: crate::VERSION,
        document: TerminalDocument { lines },
        diagnostics,
    }
}
