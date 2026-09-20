//! Rendering a document for the terminal, and writing the style out as ANSI.

use adocweave_core::output::terminal::{
    TerminalDocument, TerminalPolicy, TerminalRole, TerminalSpan, TerminalStyle, TerminalWidth,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Options {
    /// The width the reader asked for, if they asked.
    pub(crate) width: Option<u16>,
}

/// What this run of the program can show.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Capabilities {
    pub(crate) width: u16,
    pub(crate) color: bool,
}

pub(crate) fn build_policy(capabilities: Capabilities) -> TerminalPolicy {
    TerminalPolicy {
        width: TerminalWidth::Columns(capabilities.width),
        ..TerminalPolicy::default()
    }
}

pub(crate) fn render_analysis(
    analysis: &adocweave_core::Analysis,
    policy: &TerminalPolicy,
) -> TerminalDocument {
    adocweave_core::output::terminal::render(analysis.document(), policy).document
}

/// Writes the laid-out document out, with or without style.
///
/// Without color the text is the plain layout, which reads on its own. With
/// color every span is wrapped in the sequence for its role. The two differ
/// only in the escape sequences: the lines and the columns are the same.
pub(crate) fn serialize(document: &TerminalDocument, color: bool) -> String {
    if !color {
        return document.to_plain_text();
    }
    let mut output = String::new();
    for line in &document.lines {
        for span in &line.spans {
            write_span(&mut output, span);
        }
        output.push('\n');
    }
    output
}

fn write_span(output: &mut String, span: &TerminalSpan) {
    let parameters = parameters(span.style);
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

    #[test]
    fn plain_output_carries_no_escape_sequences() {
        assert_eq!(serialize(&document(), false), "Title plain\n");
    }

    /// Removing the escape sequences from the colored output gives exactly the
    /// plain output, so nothing but style depends on the choice.
    #[test]
    fn color_adds_style_and_changes_nothing_else() {
        let colored = serialize(&document(), true);

        assert!(colored.contains("\u{1b}[1;36mTitle\u{1b}[0m"));
        assert_eq!(strip(&colored), serialize(&document(), false));
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
