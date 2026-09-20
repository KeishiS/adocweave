//! Rendering a document for the terminal, and writing the style out as ANSI.

use adocweave_core::output::terminal::{
    LinkPresentation, TerminalDocument, TerminalPolicy, TerminalSpan, TerminalStyle, TerminalWidth,
};

use crate::highlight::{CodeColors, code_colors};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Options {
    /// The width the reader asked for, if they asked.
    pub(crate) width: Option<u16>,
    pub(crate) pager: crate::arguments::PagerChoice,
    pub(crate) hyperlinks: crate::arguments::HyperlinkChoice,
    pub(crate) theme: Option<crate::arguments::ThemeChoice>,
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
#[derive(Clone, Copy, Debug)]
pub(crate) struct Decoration<'theme> {
    pub(crate) color: bool,
    pub(crate) theme: &'theme crate::theme::Theme,
    /// Whether a link is written so the terminal can make it followable.
    pub(crate) hyperlinks: bool,
}

/// Writes the laid-out document out, with or without style.
///
/// Without either kind of decoration the text is the plain layout, which reads
/// on its own. The decorated form differs only in its escape sequences: the
/// lines and the columns are the same.
pub(crate) fn serialize(document: &TerminalDocument, decoration: Decoration<'_>) -> String {
    if !decoration.color && !decoration.hyperlinks {
        return document.to_plain_text();
    }
    let mut output = String::new();
    // Code is colored by the language the document named for it, which is
    // read a block at a time rather than a line at a time.
    let colors = if decoration.color {
        code_colors(document)
    } else {
        CodeColors::default()
    };
    for (index, line) in document.lines.iter().enumerate() {
        for span in &line.spans {
            match colors.line(index).filter(|_| span.language.is_some()) {
                Some(tokens) => write_code(&mut output, span, tokens, decoration),
                None => write_span(&mut output, span, decoration),
            }
        }
        output.push('\n');
    }
    output
}

/// Writes one line of code, each piece in the color of what it is.
fn write_code(
    output: &mut String,
    span: &TerminalSpan,
    tokens: &[crate::highlight::Token],
    decoration: Decoration<'_>,
) {
    for token in tokens {
        let color = decoration.theme.code_color(token.role);
        write_parameters(output, &parameters_for(span.style, color), &token.text);
    }
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

fn write_span(output: &mut String, span: &TerminalSpan, decoration: Decoration<'_>) {
    let target = decoration
        .hyperlinks
        .then_some(span.link.as_deref())
        .flatten();
    if let Some(target) = target {
        write_hyperlink_start(output, target);
    }
    write_styled_text(output, span, decoration);
    if target.is_some() {
        write_hyperlink_end(output);
    }
}

fn write_styled_text(output: &mut String, span: &TerminalSpan, decoration: Decoration<'_>) {
    let parameters = if decoration.color {
        parameters(span.style, decoration.theme)
    } else {
        Vec::new()
    };
    write_parameters(output, &parameters, &span.text);
}

fn write_parameters(output: &mut String, parameters: &[&str], text: &str) {
    if parameters.is_empty() {
        output.push_str(text);
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
    output.push_str(text);
    output.push_str("\u{1b}[0m");
}

/// The emphasis of the block, and the color of the piece inside it.
fn parameters_for(style: TerminalStyle, color: Option<&'static str>) -> Vec<&'static str> {
    let mut parameters = emphasis(style);
    if let Some(color) = color {
        parameters.push(color);
    }
    parameters
}

/// The SGR parameters for one style.
///
/// Only the eight original colors and the oldest attributes are used. A
/// terminal that shows anything at all shows these, and a reader who set a
/// palette sees the document in it.
fn parameters(style: TerminalStyle, theme: &crate::theme::Theme) -> Vec<&'static str> {
    parameters_for(style, theme.color(style.role))
}

/// Everything about a style except its color.
fn emphasis(style: TerminalStyle) -> Vec<&'static str> {
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
    parameters
}

#[cfg(test)]
mod tests {
    use adocweave_core::output::terminal::{TerminalLine, TerminalRole};

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

    fn plain(theme: &crate::theme::Theme) -> Decoration<'_> {
        Decoration {
            color: false,
            theme,
            hyperlinks: false,
        }
    }

    fn colored(theme: &crate::theme::Theme) -> Decoration<'_> {
        Decoration {
            color: true,
            theme,
            hyperlinks: false,
        }
    }

    #[test]
    fn plain_output_carries_no_escape_sequences() {
        let theme = crate::theme::Theme::default();

        assert_eq!(serialize(&document(), plain(&theme)), "Title plain\n");
    }

    /// Removing the escape sequences from the colored output gives exactly the
    /// plain output, so nothing but style depends on the choice.
    #[test]
    fn color_adds_style_and_changes_nothing_else() {
        let theme = crate::theme::Theme::default();
        let output = serialize(&document(), colored(&theme));

        assert!(output.contains("\u{1b}[1;36mTitle\u{1b}[0m"));
        assert_eq!(strip(&output), serialize(&document(), plain(&theme)));
    }

    /// A link is written around the text, whether or not the text has style.
    #[test]
    fn a_followable_link_surrounds_the_text_it_leads_from() {
        let document = TerminalDocument {
            lines: vec![TerminalLine {
                spans: vec![TerminalSpan {
                    link: Some("https://example.com".to_owned()),
                    ..TerminalSpan::new("the site", TerminalStyle::of(TerminalRole::Link))
                }],
            }],
        };
        let theme = crate::theme::Theme::default();
        let decoration = Decoration {
            color: false,
            theme: &theme,
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
