//! Column measurement, line breaking, and the buffer lines are written into.

use unicode_width::UnicodeWidthChar as _;

use super::inline::InlineUnit;
use super::{AmbiguousWidth, TerminalLine, TerminalPolicy, TerminalSpan, TerminalStyle};

/// The narrowest content a block is laid out for.
///
/// Indentation is allowed to push a block past the requested width rather than
/// squeeze its text into a column or two. A deeply nested list item that
/// overflows by a few columns still reads; the same item wrapped at three
/// columns does not.
const MIN_CONTENT_WIDTH: usize = 20;

/// How many columns `character` occupies.
pub(super) fn character_width(character: char, ambiguous: AmbiguousWidth) -> usize {
    match ambiguous {
        AmbiguousWidth::Narrow => character.width(),
        AmbiguousWidth::Wide => character.width_cjk(),
    }
    .unwrap_or(0)
}

/// Characters that may not open a line.
///
/// Japanese typesetting forbids a line that begins with a closing bracket, a
/// full stop, or a small kana, because the mark belongs to the character before
/// it. The rule is applied by moving the break one character earlier, which is
/// what a reader expects and what costs nothing to do.
fn forbidden_at_line_start(character: char) -> bool {
    matches!(
        character,
        '。' | '、'
            | '，'
            | '．'
            | '・'
            | '：'
            | '；'
            | '？'
            | '！'
            | '）'
            | '」'
            | '』'
            | '】'
            | '〕'
            | '》'
            | '〉'
            | '”'
            | '’'
            | 'ー'
            | '々'
            | '〜'
            | '…'
            | '‥'
            | 'ぁ'
            | 'ぃ'
            | 'ぅ'
            | 'ぇ'
            | 'ぉ'
            | 'っ'
            | 'ゃ'
            | 'ゅ'
            | 'ょ'
            | 'ゎ'
            | 'ァ'
            | 'ィ'
            | 'ゥ'
            | 'ェ'
            | 'ォ'
            | 'ッ'
            | 'ャ'
            | 'ュ'
            | 'ョ'
            | 'ヮ'
            | 'ヵ'
            | 'ヶ'
    )
}

/// Characters that may not close a line, for the mirror-image reason.
fn forbidden_at_line_end(character: char) -> bool {
    matches!(
        character,
        '（' | '「' | '『' | '【' | '〔' | '《' | '〈' | '“' | '‘'
    )
}

/// One base character together with the marks drawn on top of it.
struct Cluster {
    text: String,
    style: TerminalStyle,
    link: Option<String>,
    width: usize,
    base: char,
}

impl Cluster {
    const fn is_space(&self) -> bool {
        self.base == ' '
    }
}

fn clusters(spans: &[TerminalSpan], ambiguous: AmbiguousWidth) -> Vec<Cluster> {
    let mut clusters: Vec<Cluster> = Vec::new();
    for span in spans {
        for character in span.text.chars() {
            // A tab has no fixed width on a terminal, and a control character
            // would move the cursor. Neither may reach a rendered line.
            let character = if character == '\t' { ' ' } else { character };
            if character.is_control() {
                continue;
            }
            let width = character_width(character, ambiguous);
            let continues_previous = width == 0
                && clusters.last().is_some_and(|last| {
                    last.style == span.style && last.link == span.link && !last.is_space()
                });
            if continues_previous {
                let last = clusters.last_mut().expect("checked above");
                last.text.push(character);
            } else {
                clusters.push(Cluster {
                    text: character.to_string(),
                    style: span.style,
                    link: span.link.clone(),
                    width,
                    base: character,
                });
            }
        }
    }
    clusters
}

/// Whether a line may end before `clusters[index]`.
fn breakable_before(clusters: &[Cluster], index: usize) -> bool {
    if index == 0 || index >= clusters.len() {
        return false;
    }
    let before = &clusters[index - 1];
    let after = &clusters[index];
    if after.is_space() {
        return false;
    }
    if before.is_space() {
        return true;
    }
    // Text written without spaces between words breaks between characters.
    crate::cjk::is_cjk(before.base)
        && crate::cjk::is_cjk(after.base)
        && !forbidden_at_line_start(after.base)
        && !forbidden_at_line_end(before.base)
}

fn merge(clusters: &[Cluster]) -> Vec<TerminalSpan> {
    let mut spans: Vec<TerminalSpan> = Vec::new();
    for cluster in clusters {
        match spans.last_mut() {
            Some(last) if last.style == cluster.style && last.link == cluster.link => {
                last.text.push_str(&cluster.text);
            }
            // Text that is wrapped is prose. Code keeps the lines the author
            // wrote and never reaches this, so no language is carried here.
            _ => spans.push(TerminalSpan {
                text: cluster.text.clone(),
                style: cluster.style,
                link: cluster.link.clone(),
                language: None,
            }),
        }
    }
    spans
}

fn trim_end(clusters: &[Cluster]) -> &[Cluster] {
    let mut end = clusters.len();
    while end > 0 && clusters[end - 1].is_space() {
        end -= 1;
    }
    &clusters[..end]
}

/// Breaks `spans` into lines no wider than `width`.
///
/// A line ends at the last place it may, which is a space or a boundary between
/// two characters of a script written without spaces. A word longer than the
/// whole width is broken where the width runs out, because the alternative is a
/// line that overflows by an unbounded amount.
pub(super) fn wrap_spans(
    spans: &[TerminalSpan],
    width: Option<usize>,
    ambiguous: AmbiguousWidth,
) -> Vec<Vec<TerminalSpan>> {
    let clusters = clusters(spans, ambiguous);
    let Some(width) = width else {
        return vec![merge(&clusters)];
    };
    let mut lines = Vec::new();
    let mut start = 0;
    let mut used = 0;
    let mut last_break = None;
    for index in 0..clusters.len() {
        if breakable_before(&clusters, index) && index > start {
            last_break = Some(index);
        }
        let cluster_width = clusters[index].width;
        if index > start && used + cluster_width > width {
            let cut = last_break.filter(|cut| *cut > start).unwrap_or(index);
            lines.push(merge(trim_end(&clusters[start..cut])));
            start = cut;
            used = clusters[start..index].iter().map(|one| one.width).sum();
            last_break = None;
        }
        used += cluster_width;
    }
    lines.push(merge(trim_end(&clusters[start..])));
    lines
}

/// Breaks the planned inline text into lines, ending a line wherever the author
/// wrote a hard break.
pub(super) fn wrap_units(
    units: &[InlineUnit],
    width: Option<usize>,
    ambiguous: AmbiguousWidth,
) -> Vec<Vec<TerminalSpan>> {
    let mut lines = Vec::new();
    let mut spans = Vec::new();
    for unit in units {
        match unit {
            InlineUnit::Span(span) => spans.push(span.clone()),
            InlineUnit::HardBreak => {
                lines.extend(wrap_spans(&spans, width, ambiguous));
                spans.clear();
            }
        }
    }
    lines.extend(wrap_spans(&spans, width, ambiguous));
    lines
}

/// The lines rendered so far, plus what the next line starts with.
///
/// A line begins with the indentation, then the border of every block it is
/// inside, then the marker of a list item when it is the item's first line.
pub(super) struct Canvas<'policy> {
    policy: &'policy TerminalPolicy,
    lines: Vec<TerminalLine>,
    indent: usize,
    border: Vec<TerminalSpan>,
    border_width: usize,
    /// The marker the next line starts with, and how wide it is. The current
    /// indentation already holds room for it.
    marker: Option<(Vec<TerminalSpan>, usize)>,
    /// Whether the last line carries no text of its own, border aside.
    blank_last_line: bool,
}

/// Removes the spacing a border or a marker ends with, so no line ends in
/// trailing whitespace.
fn trim_trailing_spaces(spans: &mut Vec<TerminalSpan>) {
    while let Some(last) = spans.last_mut() {
        while last.text.ends_with(' ') {
            last.text.pop();
        }
        if last.text.is_empty() {
            spans.pop();
        } else {
            break;
        }
    }
}

impl<'policy> Canvas<'policy> {
    pub(super) const fn new(policy: &'policy TerminalPolicy) -> Self {
        Self {
            policy,
            lines: Vec::new(),
            indent: 0,
            border: Vec::new(),
            border_width: 0,
            marker: None,
            blank_last_line: false,
        }
    }

    pub(super) const fn policy(&self) -> &TerminalPolicy {
        self.policy
    }

    /// How many columns the text of the current block may use.
    pub(super) fn content_width(&self) -> Option<usize> {
        match self.policy.width {
            super::TerminalWidth::Unlimited => None,
            super::TerminalWidth::Columns(columns) => {
                let columns = usize::from(columns);
                Some(
                    columns
                        .saturating_sub(self.indent + self.border_width)
                        .max(MIN_CONTENT_WIDTH.min(columns)),
                )
            }
        }
    }

    /// The width of `spans` in terminal columns.
    pub(super) fn width_of(&self, spans: &[TerminalSpan]) -> usize {
        spans
            .iter()
            .map(|span| super::display_width(&span.text, self.policy.ambiguous_width))
            .sum()
    }

    /// Renders `body` with `columns` more indentation.
    pub(super) fn indented<R>(&mut self, columns: usize, body: impl FnOnce(&mut Self) -> R) -> R {
        self.indent += columns;
        let result = body(self);
        self.indent -= columns;
        result
    }

    /// Renders `body` inside a border that every one of its lines starts with.
    ///
    /// The border is what makes a quotation, an admonition, or a code block
    /// recognizable when no styling is shown at all.
    pub(super) fn bordered<R>(
        &mut self,
        border: Vec<TerminalSpan>,
        body: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let width = self.width_of(&border);
        let outer_width = self.border_width;
        let outer = std::mem::take(&mut self.border);
        self.border = outer.iter().cloned().chain(border).collect();
        self.border_width = outer_width + width;
        // The first block inside a border starts against its top, not after a
        // blank line.
        let start = self.lines.len();
        let outer_blank = std::mem::replace(&mut self.blank_last_line, true);
        let result = body(self);
        self.border = outer;
        self.border_width = outer_width;
        if self.lines.len() == start {
            self.blank_last_line = outer_blank;
        }
        result
    }

    /// Renders `body` behind `marker`, which stands in front of its first line.
    /// Every line after that begins where the first line's text begins, so an
    /// item reads as one block.
    pub(super) fn hanging<R>(
        &mut self,
        marker: Vec<TerminalSpan>,
        body: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let width = self.width_of(&marker);
        self.marker = Some((marker, width));
        let start = self.lines.len();
        self.blank_last_line = true;
        let result = self.indented(width, body);
        if self.lines.len() == start {
            // A marker is never dropped, even when the item has no text.
            self.indented(width, Self::blank_line);
        }
        self.marker = None;
        result
    }

    /// Treats the last line as a separator of its own, so what follows sits
    /// directly under it. A block title belongs to the block below it, not to
    /// a line of its own with a gap on either side.
    pub(super) const fn keep_tight(&mut self) {
        self.blank_last_line = true;
    }

    /// Separates the next block from the one before it. Does nothing at the
    /// start of the document or of a bordered block, so nothing begins with a
    /// blank line.
    pub(super) fn separate(&mut self) {
        if !self.lines.is_empty() && !self.blank_last_line {
            self.blank_line();
        }
    }

    /// Appends a line with no text of its own. Inside a bordered block the
    /// border still runs down it, which is what keeps the block one shape.
    pub(super) fn blank_line(&mut self) {
        self.emit(Vec::new());
    }

    /// Appends one line, behind the indentation, the border, and the marker.
    pub(super) fn push_line(&mut self, spans: Vec<TerminalSpan>) {
        self.emit(spans);
    }

    fn emit(&mut self, content: Vec<TerminalSpan>) {
        let blank = content.iter().all(|span| span.text.is_empty());
        let marker = self.marker.take();
        let indent = self
            .indent
            .saturating_sub(marker.as_ref().map_or(0, |(_, width)| *width));
        let mut spans = Vec::new();
        if indent > 0 {
            spans.push(TerminalSpan::new(
                " ".repeat(indent),
                TerminalStyle::default(),
            ));
        }
        spans.extend(self.border.iter().cloned());
        if let Some((marker, _)) = marker {
            spans.extend(marker);
        }
        spans.extend(content);
        if blank {
            // Nothing follows, so whatever spacing the border and the marker
            // end with would become trailing whitespace.
            trim_trailing_spaces(&mut spans);
        }
        self.blank_last_line = blank;
        spans.retain(|span| !span.text.is_empty());
        self.lines.push(TerminalLine { spans });
    }

    pub(super) fn push_text(&mut self, text: &str, style: TerminalStyle) {
        self.push_line(vec![TerminalSpan::new(text, style)]);
    }

    /// Appends the planned inline text, wrapped to the content width.
    pub(super) fn push_wrapped(&mut self, units: &[InlineUnit]) {
        let width = self.content_width();
        for line in wrap_units(units, width, self.policy.ambiguous_width) {
            self.push_line(line);
        }
    }

    /// Appends a horizontal rule of `columns` columns, never wider than the
    /// content width.
    pub(super) fn push_rule(&mut self, character: char, columns: usize, style: TerminalStyle) {
        let columns = match self.content_width() {
            Some(width) => columns.min(width),
            None => columns,
        };
        if columns == 0 {
            return;
        }
        self.push_text(&character.to_string().repeat(columns), style);
    }

    pub(super) fn finish(self) -> Vec<TerminalLine> {
        self.lines
    }
}
