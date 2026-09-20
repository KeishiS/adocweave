//! Inline text planned as styled spans.

use crate::inline_model::{
    Inline, InlineLiteralKind, InlineStyle, Link, MacroForm, Reference, StandardMacro,
    StandardMacroKind,
};
use crate::reference::{ReferenceKey, ResolutionOutcome};
use crate::render::ResolutionMatch;
use crate::url::UrlProvenance;

use super::context::RenderContext;
use super::{
    AmbiguousWidth, LinkPresentation, MathPresentation, MediaPresentation, TerminalRole,
    TerminalSpan, TerminalStyle, UnresolvedReferenceText, display_width,
};

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
pub(super) fn plan(
    inlines: &[Inline],
    style: TerminalStyle,
    context: &mut RenderContext<'_, '_>,
) -> Vec<InlineUnit> {
    let mut units = Vec::new();
    plan_sequence(inlines, style, context, &mut units);
    units
}

/// Plans text that carries no inline markup, such as a document attribute.
pub(super) fn plan_text(value: &str, style: TerminalStyle) -> Vec<InlineUnit> {
    let mut units = Vec::new();
    push_text(&mut units, value, style);
    units
}

fn plan_sequence(
    inlines: &[Inline],
    style: TerminalStyle,
    context: &mut RenderContext<'_, '_>,
    units: &mut Vec<InlineUnit>,
) {
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
            } => plan_styled(*inline_style, children, style, context, units),
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
                plan_link(link, style, context, units);
            }
            Inline::Reference(reference) => {
                drop_demanded_space_between_cjk(units);
                plan_reference(reference, style, context, units);
            }
            Inline::Macro(node) => {
                drop_demanded_space_between_cjk(units);
                plan_macro(node, style, context, units);
            }
            Inline::Formula(formula) => {
                if context.policy.math == MathPresentation::Source {
                    push_text(
                        units,
                        &formula.value,
                        TerminalStyle {
                            role: TerminalRole::Math,
                            ..style
                        },
                    );
                }
            }
            Inline::Passthrough { value, .. } => push_text(units, value, style),
            Inline::HardBreak { .. } => units.push(InlineUnit::HardBreak),
        }
    }
}

fn plan_styled(
    inline_style: InlineStyle,
    children: &[Inline],
    style: TerminalStyle,
    context: &mut RenderContext<'_, '_>,
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
            plan_sequence(children, style, context, units);
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
            plan_sequence(children, style, context, units);
        }
        InlineStyle::Strong => plan_sequence(
            children,
            TerminalStyle {
                bold: true,
                ..style
            },
            context,
            units,
        ),
        InlineStyle::Emphasis => plan_sequence(
            children,
            TerminalStyle {
                italic: true,
                ..style
            },
            context,
            units,
        ),
        InlineStyle::Highlight => plan_sequence(
            children,
            TerminalStyle {
                inverse: true,
                ..style
            },
            context,
            units,
        ),
    }
}

/// A link is its text, and then the address when the text does not already say
/// it. A terminal cannot be clicked: an address the reader cannot see is an
/// address the reader cannot follow.
fn plan_link(
    link: &Link,
    style: TerminalStyle,
    context: &mut RenderContext<'_, '_>,
    units: &mut Vec<InlineUnit>,
) {
    // Printing an address is harmless whatever it is, so every link keeps its
    // text. The policy decides only whether a host may turn it into something
    // the reader can follow.
    let active = context
        .policy
        .active_urls
        .allows(&link.target, UrlProvenance::Authored);
    let style = TerminalStyle {
        role: TerminalRole::Link,
        underline: true,
        ..style
    };
    let start = units.len();
    if link.label.is_empty() {
        push_text(units, &link.target_source, style);
    } else {
        plan_sequence(&link.label, style, context, units);
        if context.policy.links == LinkPresentation::TextWithUrl {
            push_text(
                units,
                &format!(" ({})", link.target),
                TerminalStyle {
                    role: TerminalRole::Muted,
                    ..style
                },
            );
        }
    }
    if active {
        attach_link(units, start, &link.target);
    }
}

fn plan_reference(
    reference: &Reference,
    style: TerminalStyle,
    context: &mut RenderContext<'_, '_>,
    units: &mut Vec<InlineUnit>,
) {
    let style = TerminalStyle {
        role: TerminalRole::Reference,
        ..style
    };
    match resolve_reference(reference, context) {
        Some(text) => {
            if reference.label.is_empty() {
                push_text(units, &text, style);
            } else {
                plan_sequence(&reference.label, style, context, units);
            }
        }
        None => {
            let style = TerminalStyle {
                role: TerminalRole::UnresolvedReference,
                ..style
            };
            match context.policy.unresolved_references {
                UnresolvedReferenceText::Target => {
                    if reference.label.is_empty() {
                        push_text(units, &reference.target_source, style);
                    } else {
                        plan_sequence(&reference.label, style, context, units);
                    }
                }
                UnresolvedReferenceText::LabelOnly => {
                    plan_sequence(&reference.label, style, context, units);
                }
                UnresolvedReferenceText::Hidden => {}
            }
        }
    }
}

/// The text a reference stands for, or nothing when it points nowhere.
fn resolve_reference(reference: &Reference, context: &mut RenderContext<'_, '_>) -> Option<String> {
    match &reference.target {
        Some(ReferenceKey::Local { anchor }) => match context.identifiers.target_by_id(anchor) {
            Some(target) => Some(target.label.clone()),
            None => {
                context.report(
                    "unresolved-cross-reference",
                    "local anchor does not exist",
                    reference.target_range,
                );
                None
            }
        },
        None => {
            context.report(
                "invalid-cross-reference",
                "invalid cross reference target",
                reference.target_range,
            );
            None
        }
        // A reference into another document is resolved by the host, which
        // knows where that document went.
        Some(ReferenceKey::Document { .. } | ReferenceKey::Scheme { .. }) => {
            match context.usage.reference_at(reference.range) {
                ResolutionMatch::Unique(resolution) => match &resolution.outcome {
                    ResolutionOutcome::Resolved { display_text, .. } => Some(
                        display_text
                            .clone()
                            .unwrap_or_else(|| reference.target_source.clone()),
                    ),
                    ResolutionOutcome::Failed(_) => {
                        context.report(
                            "unresolved-cross-reference",
                            "reference target does not exist",
                            reference.target_range,
                        );
                        None
                    }
                },
                ResolutionMatch::Missing | ResolutionMatch::Duplicate => {
                    context.report(
                        "unresolved-cross-reference",
                        "reference target was not resolved by the host",
                        reference.target_range,
                    );
                    None
                }
            }
        }
    }
}

fn plan_macro(
    node: &StandardMacro,
    style: TerminalStyle,
    context: &mut RenderContext<'_, '_>,
    units: &mut Vec<InlineUnit>,
) {
    let first = node
        .attributes
        .first()
        .map(|attribute| attribute.value.as_str());
    match node.kind {
        StandardMacroKind::Email => {
            let style = TerminalStyle {
                role: TerminalRole::Link,
                underline: true,
                ..style
            };
            let start = units.len();
            push_text(units, &node.target, style);
            attach_link(units, start, &format!("mailto:{}", node.target));
        }
        // The number is what ties the mark to the note at the end. Without the
        // catalog there is no number, so the text itself stands in.
        StandardMacroKind::Footnote => match context.catalogs.footnote_occurrence(node.range) {
            Some((footnote, _)) => push_text(
                units,
                &format!("[{}]", footnote.number),
                TerminalStyle {
                    role: TerminalRole::FootnoteMarker,
                    ..style
                },
            ),
            None => push_text(units, first.unwrap_or(&node.target), style),
        },
        // An anchor and an index term are landing points, not text.
        StandardMacroKind::Anchor
        | StandardMacroKind::BibliographyAnchor
        | StandardMacroKind::IndexTerm => {}
        StandardMacroKind::Citation => plan_citation(node, style, context, units),
        StandardMacroKind::Keyboard | StandardMacroKind::Button => push_text(
            units,
            first.unwrap_or(&node.target),
            TerminalStyle {
                role: TerminalRole::Monospace,
                ..style
            },
        ),
        StandardMacroKind::Menu => {
            let mut text = node.target.clone();
            for attribute in &node.attributes {
                text.push_str(" › ");
                text.push_str(&attribute.value);
            }
            push_text(units, &text, style);
        }
        StandardMacroKind::Image | StandardMacroKind::Icon => {
            plan_media(node, "Image", style, context, units);
        }
        StandardMacroKind::Audio => plan_media(node, "Audio", style, context, units),
        StandardMacroKind::Video => plan_media(node, "Video", style, context, units),
    }
}

/// A citation shows the label of the entry it names, or the key itself when
/// this document defines no such entry.
fn plan_citation(
    node: &StandardMacro,
    style: TerminalStyle,
    context: &mut RenderContext<'_, '_>,
    units: &mut Vec<InlineUnit>,
) {
    let style = TerminalStyle {
        role: TerminalRole::Reference,
        ..style
    };
    let keys: Vec<&crate::inline_model::MacroAttribute> = node
        .attributes
        .iter()
        .filter(|attribute| attribute.name.is_none())
        .collect();
    let mut separator = "";
    for key in keys {
        let entry = context
            .catalogs
            .bibliography()
            .iter()
            .find(|entry| entry.id == key.value);
        let text = match entry {
            Some(entry) => entry.label.clone().unwrap_or_else(|| key.value.clone()),
            None => match context.policy.unresolved_references {
                UnresolvedReferenceText::Target | UnresolvedReferenceText::LabelOnly => {
                    key.value.clone()
                }
                UnresolvedReferenceText::Hidden => continue,
            },
        };
        push_text(units, separator, style);
        push_text(units, &text, style);
        separator = ", ";
    }
}

/// A terminal shows no pictures and plays no sound. What it can say is that
/// something is there and what it is called.
fn plan_media(
    node: &StandardMacro,
    kind: &str,
    style: TerminalStyle,
    context: &mut RenderContext<'_, '_>,
    units: &mut Vec<InlineUnit>,
) {
    if context.policy.media == MediaPresentation::Hidden {
        return;
    }
    let described = node
        .attributes
        .iter()
        .find(|attribute| {
            attribute.name.as_deref() == Some("alt") || attribute.name.as_deref() == Some("title")
        })
        .or_else(|| {
            node.attributes
                .iter()
                .find(|attribute| attribute.name.is_none())
        })
        .map(|attribute| attribute.value.as_str())
        .filter(|value| !value.is_empty());
    let text = match described {
        Some(description) => format!("[{kind}: {description}]"),
        None => format!("[{kind}: {}]", node.target),
    };
    let mut style = TerminalStyle {
        role: TerminalRole::MediaPlaceholder,
        ..style
    };
    if node.form == MacroForm::Block {
        style.bold = false;
    }
    push_text(units, &text, style);
}

/// Gives every span planned since `start` the address it points at.
fn attach_link(units: &mut [InlineUnit], start: usize, target: &str) {
    for unit in &mut units[start..] {
        if let InlineUnit::Span(span) = unit
            && span.link.is_none()
        {
            span.link = Some(target.to_owned());
        }
    }
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
