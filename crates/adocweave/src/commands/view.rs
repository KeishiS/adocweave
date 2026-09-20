//! Rendering a document for the terminal, and writing the style out as ANSI.

use adocweave_core::output::terminal::{
    LinkPresentation, TerminalDocument, TerminalPolicy, TerminalRole, TerminalSpan, TerminalStyle,
    TerminalWidth,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Options {
    /// The width the reader asked for, if they asked.
    pub(crate) width: Option<u16>,
    pub(crate) pager: crate::arguments::PagerChoice,
    pub(crate) hyperlinks: crate::arguments::HyperlinkChoice,
}

pub(crate) fn build_policy(width: u16, hyperlinks: bool) -> TerminalPolicy {
    TerminalPolicy {
        width: TerminalWidth::Columns(width),
        // A terminal that can make the text itself followable has no use for
        // the address written after it.
        links: if hyperlinks {
            LinkPresentation::TextWhenFollowable
        } else {
            LinkPresentation::TextWithUrl
        },
        ..TerminalPolicy::default()
    }
}

pub(crate) fn render_analysis(
    analysis: &adocweave_core::Analysis,
    policy: &TerminalPolicy,
) -> TerminalDocument {
    adocweave_core::output::terminal::render(analysis.document(), policy).document
}

/// What the reader's terminal is given beyond the text itself.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Decoration {
    pub(crate) color: bool,
    /// Whether a link is written so the terminal can make it followable.
    pub(crate) hyperlinks: bool,
}

/// Writes the laid-out document out, with or without style.
///
/// Without either kind of decoration the text is the plain layout, which reads
/// on its own. The decorated form differs only in its escape sequences: the
/// lines and the columns are the same.
pub(crate) fn serialize(document: &TerminalDocument, decoration: Decoration) -> String {
    if !decoration.color && !decoration.hyperlinks {
        return document.to_plain_text();
    }
    let mut output = String::new();
    for line in &document.lines {
        for span in &line.spans {
            write_span(&mut output, span, decoration);
        }
        output.push('\n');
    }
    output
}

/// The start and end of a followable link, as OSC 8 writes them. A terminal
/// that does not know the sequence shows nothing for it.
fn write_hyperlink_start(output: &mut String, target: &str) {
    output.push_str("\u{1b}]8;;");
    output.push_str(target);
    output.push_str("\u{1b}\\");
}

fn write_hyperlink_end(output: &mut String) {
    output.push_str("\u{1b}]8;;\u{1b}\\");
}

fn write_span(output: &mut String, span: &TerminalSpan, decoration: Decoration) {
    let target = decoration
        .hyperlinks
        .then_some(span.link.as_deref())
        .flatten();
    if let Some(target) = target {
        write_hyperlink_start(output, target);
    }
    write_styled_text(output, span, decoration.color);
    if target.is_some() {
        write_hyperlink_end(output);
    }
}

fn write_styled_text(output: &mut String, span: &TerminalSpan, color: bool) {
    let parameters = if color {
        parameters(span.style)
    } else {
        Vec::new()
    };
    if parameters.is_empty() {
        output.push_str(&span.text);
        return;
    }
    output.push_str("\u{1b}[");
    for (index, parameter) in parameters.iter().enumerate() {
        if index > 0 {
            output.push(';');
        }
        output.push_str(parameter);
    }
    output.push('m');
    output.push_str(&span.text);
    output.push_str("\u{1b}[0m");
}

/// The SGR parameters for one style.
///
/// Only the eight original colors and the oldest attributes are used. A
/// terminal that shows anything at all shows these, and a reader who set a
/// palette sees the document in it.
fn parameters(style: TerminalStyle) -> Vec<&'static str> {
    let mut parameters = Vec::new();
    if style.bold {
        parameters.push("1");
    }
    if style.dim {
        parameters.push("2");
    }
    if style.italic {
        parameters.push("3");
    }
    if style.underline {
        parameters.push("4");
    }
    if style.inverse {
        parameters.push("7");
    }
    if let Some(color) = color(style.role) {
        parameters.push(color);
    }
    parameters
}

fn color(role: TerminalRole) -> Option<&'static str> {
    use adocweave_core::semantic::AdmonitionKind;

    Some(match role {
        TerminalRole::Text | TerminalRole::FootnoteText => return None,
        TerminalRole::DocumentTitle | TerminalRole::Heading { .. } => "36",
        TerminalRole::Metadata | TerminalRole::Muted | TerminalRole::Rule => "90",
        TerminalRole::Marker | TerminalRole::FootnoteMarker => "33",
        TerminalRole::Caption | TerminalRole::TableHeader => "36",
        TerminalRole::Monospace | TerminalRole::Code | TerminalRole::Math => "32",
        TerminalRole::Link | TerminalRole::Reference => "34",
        TerminalRole::UnresolvedReference | TerminalRole::Unsupported => "31",
        TerminalRole::Admonition(kind) => match kind {
            AdmonitionKind::Note => "36",
            AdmonitionKind::Tip => "32",
            AdmonitionKind::Important => "35",
            AdmonitionKind::Warning => "33",
            AdmonitionKind::Caution => "31",
        },
        TerminalRole::Quote | TerminalRole::Attribution => "35",
        TerminalRole::TableBorder => "90",
        TerminalRole::MediaPlaceholder => "90",
    })
}

#[cfg(test)]
mod tests {
    use adocweave_core::output::terminal::TerminalLine;

    use super::*;

    fn document() -> TerminalDocument {
        TerminalDocument {
            lines: vec![TerminalLine {
                spans: vec![
                    TerminalSpan::new(
                        "Title",
                        TerminalStyle {
                            role: TerminalRole::DocumentTitle,
                            bold: true,
                            ..TerminalStyle::default()
                        },
                    ),
                    TerminalSpan::new(" plain", TerminalStyle::default()),
                ],
            }],
        }
    }

    fn plain() -> Decoration {
        Decoration::default()
    }

    fn colored() -> Decoration {
        Decoration {
            color: true,
            hyperlinks: false,
        }
    }

    #[test]
    fn plain_output_carries_no_escape_sequences() {
        assert_eq!(serialize(&document(), plain()), "Title plain\n");
    }

    /// Removing the escape sequences from the colored output gives exactly the
    /// plain output, so nothing but style depends on the choice.
    #[test]
    fn color_adds_style_and_changes_nothing_else() {
        let output = serialize(&document(), colored());

        assert!(output.contains("\u{1b}[1;36mTitle\u{1b}[0m"));
        assert_eq!(strip(&output), serialize(&document(), plain()));
    }

    /// A link is written around the text, whether or not the text has style.
    #[test]
    fn a_followable_link_surrounds_the_text_it_leads_from() {
        let document = TerminalDocument {
            lines: vec![TerminalLine {
                spans: vec![TerminalSpan {
                    text: "the site".to_owned(),
                    style: TerminalStyle::of(TerminalRole::Link),
                    link: Some("https://example.com".to_owned()),
                }],
            }],
        };
        let decoration = Decoration {
            color: false,
            hyperlinks: true,
        };

        assert_eq!(
            serialize(&document, decoration),
            "\u{1b}]8;;https://example.com\u{1b}\\the site\u{1b}]8;;\u{1b}\\\n"
        );
    }

    fn strip(text: &str) -> String {
        let mut plain = String::new();
        let mut characters = text.chars();
        while let Some(character) = characters.next() {
            if character != '\u{1b}' {
                plain.push(character);
                continue;
            }
            for character in characters.by_ref() {
                if character == 'm' {
                    break;
                }
            }
        }
        plain
    }
}
