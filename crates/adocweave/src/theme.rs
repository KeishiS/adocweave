//! The colors a role is shown in.
//!
//! The backend says what each piece of text is; what it looks like is decided
//! here. A palette is written for the background the page is read on, and a
//! reader who wants something else names a color for the role themselves.

use std::collections::BTreeMap;

use adocweave_core::output::terminal::TerminalRole;
use adocweave_project::{TerminalColor, TerminalSettings, TerminalTheme};

/// A palette, and the colors chosen for single roles on top of it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Theme {
    palette: Palette,
    chosen: BTreeMap<String, TerminalColor>,
}

/// What one piece of code is, inside a block the reader is shown.
///
/// A terminal has few colors, so code is told apart by the handful of kinds
/// a reader actually looks for, not by every scope a language defines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CodeRole {
    /// Code with nothing else to say about it.
    Plain,
    Comment,
    /// A string, or any other text written inside the code.
    Text,
    /// A number or another value the language writes literally.
    Value,
    Keyword,
    Function,
    /// The name of a type, a class, or another thing the code declares.
    Name,
}

/// The background the palette is written for.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Palette {
    /// Colors that stand out against a dark background, where bright cyan and
    /// yellow read well.
    #[default]
    Dark,
    /// Colors that stand out against a light background, where those two wash
    /// out and the darker blue and magenta take their place.
    Light,
}

impl Theme {
    /// The theme the reader asked for, and the one the project named when they
    /// asked for none.
    pub(crate) fn resolve(named: Option<Palette>, settings: &TerminalSettings) -> Self {
        let palette = named
            .or_else(|| settings.theme.map(Palette::from))
            .unwrap_or_default();
        Self {
            palette,
            chosen: settings.colors.clone(),
        }
    }

    /// The SGR parameter for `role`, or nothing when it keeps the color the
    /// terminal was already using.
    pub(crate) fn color(&self, role: TerminalRole) -> Option<&'static str> {
        match self.chosen.get(role.name()) {
            Some(color) => parameter(*color),
            None => self.palette.color(role),
        }
    }

    /// The SGR parameter for one piece of code.
    ///
    /// Code sits inside a block that already has a color, so a piece with
    /// nothing else to say about it keeps that color, and the name of what
    /// the code declares is left in the color the terminal reads in. The
    /// rest are told apart from each other and from the block around them.
    pub(crate) fn code_color(&self, role: CodeRole) -> Option<&'static str> {
        let dark = self.palette == Palette::Dark;
        match role {
            CodeRole::Plain => self.color(TerminalRole::Code),
            CodeRole::Comment => self.color(TerminalRole::Muted),
            CodeRole::Name => None,
            CodeRole::Value => parameter(TerminalColor::Magenta),
            // Yellow reads on a dark background and washes out on a light
            // one, where red takes its place.
            CodeRole::Text => {
                if dark {
                    parameter(TerminalColor::Yellow)
                } else {
                    parameter(TerminalColor::Red)
                }
            }
            CodeRole::Keyword => {
                if dark {
                    parameter(TerminalColor::Cyan)
                } else {
                    parameter(TerminalColor::Blue)
                }
            }
            CodeRole::Function => {
                if dark {
                    parameter(TerminalColor::Blue)
                } else {
                    parameter(TerminalColor::Cyan)
                }
            }
        }
    }
}

impl From<TerminalTheme> for Palette {
    fn from(value: TerminalTheme) -> Self {
        match value {
            TerminalTheme::Dark => Self::Dark,
            TerminalTheme::Light => Self::Light,
        }
    }
}

impl Palette {
    fn color(self, role: TerminalRole) -> Option<&'static str> {
        use adocweave_core::semantic::AdmonitionKind;

        let dark = self == Self::Dark;
        Some(match role {
            TerminalRole::Text | TerminalRole::FootnoteText => return None,
            // A heading, a caption, and the head of a table name what follows
            // them, so they share one color.
            TerminalRole::DocumentTitle
            | TerminalRole::Heading { .. }
            | TerminalRole::Caption
            | TerminalRole::TableHeader => {
                if dark {
                    parameter(TerminalColor::Cyan)?
                } else {
                    parameter(TerminalColor::Blue)?
                }
            }
            // Text that recedes: the same gray reads on either background.
            TerminalRole::Metadata
            | TerminalRole::Muted
            | TerminalRole::Rule
            | TerminalRole::TableBorder
            | TerminalRole::MediaPlaceholder => parameter(TerminalColor::BrightBlack)?,
            TerminalRole::Marker | TerminalRole::FootnoteMarker => {
                if dark {
                    parameter(TerminalColor::Yellow)?
                } else {
                    parameter(TerminalColor::Magenta)?
                }
            }
            TerminalRole::Monospace | TerminalRole::Code | TerminalRole::Math => {
                parameter(TerminalColor::Green)?
            }
            TerminalRole::Link | TerminalRole::Reference => parameter(TerminalColor::Blue)?,
            TerminalRole::UnresolvedReference | TerminalRole::Unsupported => {
                parameter(TerminalColor::Red)?
            }
            TerminalRole::Admonition(kind) => match kind {
                AdmonitionKind::Note => {
                    if dark {
                        parameter(TerminalColor::Cyan)?
                    } else {
                        parameter(TerminalColor::Blue)?
                    }
                }
                AdmonitionKind::Tip => parameter(TerminalColor::Green)?,
                AdmonitionKind::Important => parameter(TerminalColor::Magenta)?,
                AdmonitionKind::Warning => {
                    if dark {
                        parameter(TerminalColor::Yellow)?
                    } else {
                        parameter(TerminalColor::Magenta)?
                    }
                }
                AdmonitionKind::Caution => parameter(TerminalColor::Red)?,
            },
            TerminalRole::Quote | TerminalRole::Attribution => parameter(TerminalColor::Magenta)?,
        })
    }
}

/// The SGR parameter that selects `color` as the foreground.
const fn parameter(color: TerminalColor) -> Option<&'static str> {
    Some(match color {
        TerminalColor::Default => return None,
        TerminalColor::Black => "30",
        TerminalColor::Red => "31",
        TerminalColor::Green => "32",
        TerminalColor::Yellow => "33",
        TerminalColor::Blue => "34",
        TerminalColor::Magenta => "35",
        TerminalColor::Cyan => "36",
        TerminalColor::White => "37",
        TerminalColor::BrightBlack => "90",
        TerminalColor::BrightRed => "91",
        TerminalColor::BrightGreen => "92",
        TerminalColor::BrightYellow => "93",
        TerminalColor::BrightBlue => "94",
        TerminalColor::BrightMagenta => "95",
        TerminalColor::BrightCyan => "96",
        TerminalColor::BrightWhite => "97",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(
        theme: Option<TerminalTheme>,
        colors: &[(&str, TerminalColor)],
    ) -> TerminalSettings {
        TerminalSettings {
            theme,
            colors: colors
                .iter()
                .map(|(name, color)| ((*name).to_owned(), *color))
                .collect(),
        }
    }

    #[test]
    fn a_palette_written_for_one_background_differs_from_the_other() {
        let dark = Theme::resolve(None, &settings(Some(TerminalTheme::Dark), &[]));
        let light = Theme::resolve(None, &settings(Some(TerminalTheme::Light), &[]));

        assert_eq!(dark.color(TerminalRole::Heading { level: 1 }), Some("36"));
        assert_eq!(light.color(TerminalRole::Heading { level: 1 }), Some("34"));
    }

    /// What the reader named on the command line stands over what the project
    /// named, which stands over the palette written for a dark background.
    #[test]
    fn the_reader_has_the_last_word_about_the_palette() {
        let project = settings(Some(TerminalTheme::Light), &[]);

        assert_eq!(
            Theme::resolve(Some(Palette::Dark), &project).color(TerminalRole::Heading { level: 1 }),
            Some("36")
        );
        assert_eq!(
            Theme::resolve(None, &TerminalSettings::default())
                .color(TerminalRole::Heading { level: 1 }),
            Some("36")
        );
    }

    #[test]
    fn a_color_chosen_for_a_role_stands_over_the_palette() {
        let theme = Theme::resolve(
            None,
            &settings(None, &[("heading", TerminalColor::BrightMagenta)]),
        );

        assert_eq!(theme.color(TerminalRole::Heading { level: 2 }), Some("95"));
        assert_eq!(theme.color(TerminalRole::Link), Some("34"));
    }

    /// A role can be given back the color the terminal was already using.
    #[test]
    fn a_role_can_be_left_in_the_color_the_terminal_was_using() {
        let theme = Theme::resolve(None, &settings(None, &[("code", TerminalColor::Default)]));

        assert_eq!(theme.color(TerminalRole::Code), None);
    }

    /// No two kinds of code share a color, and none of them is the color of
    /// the block around them, or they could not be told apart.
    #[test]
    fn the_kinds_of_code_are_told_apart_in_either_palette() {
        for theme in [TerminalTheme::Dark, TerminalTheme::Light] {
            let theme = Theme::resolve(None, &settings(Some(theme), &[]));
            let colors = [
                theme.code_color(CodeRole::Comment),
                theme.code_color(CodeRole::Text),
                theme.code_color(CodeRole::Value),
                theme.code_color(CodeRole::Keyword),
                theme.code_color(CodeRole::Function),
                theme.code_color(CodeRole::Name),
                theme.code_color(CodeRole::Plain),
            ];
            let unique: std::collections::BTreeSet<_> = colors.iter().collect();

            assert_eq!(unique.len(), colors.len(), "{colors:?}");
        }
    }

    /// Plain code follows the color chosen for a code block, so a reader who
    /// changed that sees the code change with it.
    #[test]
    fn plain_code_follows_the_color_of_the_block_it_sits_in() {
        let theme = Theme::resolve(None, &settings(None, &[("code", TerminalColor::Blue)]));

        assert_eq!(theme.code_color(CodeRole::Plain), Some("34"));
    }

    #[test]
    fn body_text_keeps_the_color_the_terminal_was_using() {
        let theme = Theme::default();

        assert_eq!(theme.color(TerminalRole::Text), None);
    }
}
