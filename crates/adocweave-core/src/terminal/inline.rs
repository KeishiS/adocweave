//! Inline text planned as styled spans.

use crate::inline_model::{Inline, InlineLiteralKind, InlineStyle};

use super::{AmbiguousWidth, TerminalRole, TerminalSpan, TerminalStyle, display_width};

/// A piece of planned inline text.
pub(super) enum InlineUnit {
    Span(TerminalSpan),
    /// A line break the author wrote. It ends the line wherever it falls.
    HardBreak,
}

/// How many columns the planned text needs when nothing wraps.
pub(super) fn units_display_width(units: &[InlineUnit], ambiguous: AmbiguousWidth) -> usize {
    let mut widest = 0;
    let mut current = 0;
    for unit in units {
        match unit {
            InlineUnit::Span(span) => current += display_width(&span.text, ambiguous),
            InlineUnit::HardBreak => {
                widest = widest.max(current);
                current = 0;
            }
        }
    }
    widest.max(current)
}

/// Plans `inlines` as spans that all carry `style` unless they change it.
pub(super) fn plan(inlines: &[Inline], style: TerminalStyle) -> Vec<InlineUnit> {
    let mut units = Vec::new();
    plan_sequence(inlines, style, &mut units);
    units
}

/// Plans text that carries no inline markup, such as a document attribute.
pub(super) fn plan_text(value: &str, style: TerminalStyle) -> Vec<InlineUnit> {
    let mut units = Vec::new();
    push_text(&mut units, value, style);
    units
}

fn plan_sequence(inlines: &[Inline], style: TerminalStyle, units: &mut Vec<InlineUnit>) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => push_text(units, &text.value, style),
            Inline::Literal { kind, value, .. } => match kind {
                InlineLiteralKind::Monospace => push_text(
                    units,
                    value,
                    TerminalStyle {
                        role: TerminalRole::Monospace,
                        ..style
                    },
                ),
            },
            Inline::Styled {
                style: inline_style,
                children,
                ..
            } => plan_styled(*inline_style, children, style, units),
            Inline::AttributeReference { name, value, .. } => match value {
                Some(value) => push_text(units, value, style),
                None => {
                    push_text(units, "{", style);
                    push_text(units, name, style);
                    push_text(units, "}", style);
                }
            },
            // A macro or a reference is the one construct an author cannot
            // write without a space in front of it, so this is where the space
            // the syntax demanded is given back, exactly as in HTML.
            Inline::Link(link) => {
                drop_demanded_space_between_cjk(units);
                let label = if link.label.is_empty() {
                    plan_text(
                        &link.target,
                        TerminalStyle {
                            role: TerminalRole::Link,
                            underline: true,
                            ..style
                        },
                    )
                } else {
                    plan(
                        &link.label,
                        TerminalStyle {
                            role: TerminalRole::Link,
                            underline: true,
                            ..style
                        },
                    )
                };
                units.extend(with_link(label, &link.target));
            }
            Inline::Reference(reference) => {
                drop_demanded_space_between_cjk(units);
                let style = TerminalStyle {
                    role: TerminalRole::Reference,
                    ..style
                };
                if reference.label.is_empty() {
                    push_text(units, &reference.expanded_target, style);
                } else {
                    plan_sequence(&reference.label, style, units);
                }
            }
            Inline::Macro(node) => {
                drop_demanded_space_between_cjk(units);
                // Until each kind of macro has a presentation of its own, a
                // macro is shown by the text it carries, so nothing an author
                // wrote disappears from the page.
                let text = node
                    .attributes
                    .first()
                    .map(|attribute| attribute.value.as_str())
                    .filter(|value| !value.is_empty())
                    .unwrap_or(node.target.as_str());
                push_text(units, text, style);
            }
            Inline::Formula(formula) => push_text(
                units,
                &formula.value,
                TerminalStyle {
                    role: TerminalRole::Math,
                    ..style
                },
            ),
            Inline::Passthrough { value, .. } => push_text(units, value, style),
            Inline::HardBreak { .. } => units.push(InlineUnit::HardBreak),
        }
    }
}

fn plan_styled(
    inline_style: InlineStyle,
    children: &[Inline],
    style: TerminalStyle,
    units: &mut Vec<InlineUnit>,
) {
    match inline_style {
        InlineStyle::CurvedDoubleQuote | InlineStyle::CurvedSingleQuote => {
            let (open, close) = if inline_style == InlineStyle::CurvedDoubleQuote {
                ("“", "”")
            } else {
                ("‘", "’")
            };
            push_text(units, open, style);
            plan_sequence(children, style, units);
            push_text(units, close, style);
        }
        // A terminal has no raised or lowered text. The marker the author typed
        // is kept instead, so `x^2^` reads as `x^2` rather than as `x2`.
        InlineStyle::Subscript | InlineStyle::Superscript => {
            let marker = if inline_style == InlineStyle::Superscript {
                "^"
            } else {
                "~"
            };
            push_text(units, marker, style);
            plan_sequence(children, style, units);
        }
        InlineStyle::Strong => plan_sequence(
            children,
            TerminalStyle {
                bold: true,
                ..style
            },
            units,
        ),
        InlineStyle::Emphasis => plan_sequence(
            children,
            TerminalStyle {
                italic: true,
                ..style
            },
            units,
        ),
        InlineStyle::Highlight => plan_sequence(
            children,
            TerminalStyle {
                inverse: true,
                ..style
            },
            units,
        ),
    }
}

fn with_link(units: Vec<InlineUnit>, target: &str) -> Vec<InlineUnit> {
    units
        .into_iter()
        .map(|unit| match unit {
            InlineUnit::Span(span) => InlineUnit::Span(TerminalSpan {
                link: Some(target.to_owned()),
                ..span
            }),
            InlineUnit::HardBreak => unit,
        })
        .collect()
}

/// Appends `value` as text, joining the lines of a paragraph the way the HTML
/// backend does: a line break becomes a space, except between two characters of
/// a script that is written without spaces between words.
fn push_text(units: &mut Vec<InlineUnit>, value: &str, style: TerminalStyle) {
    let mut text = String::new();
    let mut characters = value.chars().peekable();
    let mut previous: Option<char> = None;
    while let Some(character) = characters.next() {
        if character == '\r' || character == '\n' {
            if character == '\r' && characters.peek() == Some(&'\n') {
                characters.next();
            }
            if crate::cjk::joins_without_space(previous, characters.peek().copied()) {
                continue;
            }
            text.push(' ');
            previous = Some(' ');
        } else {
            text.push(character);
            previous = Some(character);
        }
    }
    if text.is_empty() {
        return;
    }
    match units.last_mut() {
        Some(InlineUnit::Span(span)) if span.style == style && span.link.is_none() => {
            span.text.push_str(&text);
        }
        _ => units.push(InlineUnit::Span(TerminalSpan::new(text, style))),
    }
}

/// Gives back the space an inline macro or reference demanded in front of it,
/// under the same rule the HTML backend applies.
fn drop_demanded_space_between_cjk(units: &mut [InlineUnit]) {
    let Some(InlineUnit::Span(span)) = units.last_mut() else {
        return;
    };
    let mut characters = span.text.chars().rev();
    if characters.next() != Some(' ') {
        return;
    }
    if characters.next().is_some_and(crate::cjk::is_cjk) {
        span.text.pop();
    }
}
