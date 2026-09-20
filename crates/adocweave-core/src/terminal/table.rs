//! Tables measured and drawn for a fixed number of columns.
//!
//! HTML leaves the width of a column to the browser. A terminal has no such
//! help: the width of every column has to be decided here, from the text the
//! cells hold and the room the page has left.

use crate::block_model::BlockMetadata;
use crate::table::{
    HorizontalAlignment, Table, TableCell, TableCellContent, TableCellStyle, TableFrame, TableGrid,
    TableSection, VerticalAlignment,
};

use super::context::RenderContext;
use super::layout::{Canvas, wrap_units};
use super::{
    TableBorders, TerminalPolicy, TerminalRole, TerminalSpan, TerminalStyle, blocks, display_width,
    inline,
};

/// The narrowest a column is ever squeezed to. Below this, a column holds
/// nothing a reader can use.
const MIN_COLUMN_WIDTH: usize = 3;

/// The characters a table is drawn with.
struct BorderSet {
    vertical: &'static str,
    horizontal: char,
    top_left: char,
    top_right: char,
    top_join: char,
    bottom_left: char,
    bottom_right: char,
    bottom_join: char,
    left_join: char,
    right_join: char,
    cross: char,
}

const UNICODE_BORDERS: BorderSet = BorderSet {
    vertical: "│",
    horizontal: '─',
    top_left: '┌',
    top_right: '┐',
    top_join: '┬',
    bottom_left: '└',
    bottom_right: '┘',
    bottom_join: '┴',
    left_join: '├',
    right_join: '┤',
    cross: '┼',
};

const ASCII_BORDERS: BorderSet = BorderSet {
    vertical: "|",
    horizontal: '-',
    top_left: '+',
    top_right: '+',
    top_join: '+',
    bottom_left: '+',
    bottom_right: '+',
    bottom_join: '+',
    left_join: '+',
    right_join: '+',
    cross: '+',
};

/// What this table is drawn with, after the host's setting and the document's
/// own `frame` and `grid` have both been taken into account.
struct Shape {
    borders: Option<&'static BorderSet>,
    /// Whether the outer edge and the rules above and below are drawn.
    frame: bool,
    /// Whether neighbouring columns are separated by a line.
    columns: bool,
}

impl Shape {
    fn of(policy: &TerminalPolicy, table: &Table) -> Self {
        let borders = match policy.table_borders {
            TableBorders::Unicode => Some(&UNICODE_BORDERS),
            TableBorders::Ascii => Some(&ASCII_BORDERS),
            TableBorders::None => None,
        };
        Self {
            borders,
            frame: borders.is_some() && table.presentation.frame != TableFrame::None,
            columns: borders.is_some()
                && matches!(table.presentation.grid, TableGrid::All | TableGrid::Columns),
        }
    }

    /// The columns spent on the edges and the separators of a row.
    fn furniture(&self, columns: usize) -> usize {
        let edges = if self.frame { 2 * self.edge_width() } else { 0 };
        edges + columns.saturating_sub(1) * self.separator_width()
    }

    fn edge_width(&self) -> usize {
        // The line itself and the space that keeps text off it.
        2
    }

    fn separator_width(&self) -> usize {
        if self.columns { 3 } else { 2 }
    }
}

/// One cell, together with where it sits and how far it reaches.
struct Placement<'table> {
    cell: &'table TableCell,
    row: usize,
    column: usize,
    column_span: usize,
}

pub(super) fn render(
    canvas: &mut Canvas<'_>,
    table: &Table,
    metadata: &BlockMetadata,
    range: crate::source::TextRange,
    context: &mut RenderContext<'_, '_>,
) {
    let columns = column_count(table);
    if columns == 0 || table.rows.is_empty() {
        return;
    }
    let policy = canvas.policy().clone();
    let shape = Shape::of(&policy, table);
    let placements = placements(table, columns);
    let widths = column_widths(
        &policy,
        table,
        &placements,
        columns,
        canvas,
        &shape,
        context,
    );

    canvas.separate();
    blocks::render_caption(canvas, metadata, range, context);
    let border_style = TerminalStyle::of(TerminalRole::TableBorder);

    if shape.frame
        && let Some(borders) = shape.borders
    {
        canvas.push_line(vec![TerminalSpan::new(
            rule(
                borders,
                &widths,
                &shape,
                borders.top_left,
                borders.top_join,
                borders.top_right,
                &boundaries(&placements, 0, columns),
            ),
            border_style,
        )]);
    }
    let mut section = None;
    for (index, row) in table.rows.iter().enumerate() {
        if let Some(previous) = section
            && previous != row.section
            && let Some(borders) = shape.borders
        {
            let above = boundaries(&placements, index.saturating_sub(1), columns);
            let below = boundaries(&placements, index, columns);
            let joins: Vec<bool> = above
                .iter()
                .zip(&below)
                .map(|(above, below)| *above || *below)
                .collect();
            canvas.push_line(vec![TerminalSpan::new(
                rule(
                    borders,
                    &widths,
                    &shape,
                    borders.left_join,
                    borders.cross,
                    borders.right_join,
                    &joins,
                ),
                border_style,
            )]);
        }
        section = Some(row.section);
        render_row(
            canvas,
            &policy,
            table,
            &placements,
            index,
            columns,
            &widths,
            &shape,
            context,
        );
    }
    if shape.frame
        && let Some(borders) = shape.borders
    {
        canvas.push_line(vec![TerminalSpan::new(
            rule(
                borders,
                &widths,
                &shape,
                borders.bottom_left,
                borders.bottom_join,
                borders.bottom_right,
                &boundaries(&placements, table.rows.len() - 1, columns),
            ),
            border_style,
        )]);
    }
}

/// Where the cells of one row end, so a line drawn next to that row knows
/// which boundaries are real.
fn boundaries(placements: &[Placement<'_>], row: usize, columns: usize) -> Vec<bool> {
    (0..columns.saturating_sub(1))
        .map(|index| {
            !placements.iter().any(|placement| {
                placement.row == row
                    && placement.column <= index
                    && placement.column + placement.column_span >= index + 2
            })
        })
        .collect()
}

fn column_count(table: &Table) -> usize {
    let declared = table.columns.len();
    let used = table
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .map(|cell| cell.column_index as usize + cell.column_span.max(1) as usize)
        .max()
        .unwrap_or(0);
    declared.max(used)
}

fn placements<'table>(table: &'table Table, columns: usize) -> Vec<Placement<'table>> {
    let mut placements = Vec::new();
    for (index, row) in table.rows.iter().enumerate() {
        for cell in &row.cells {
            let column = (cell.column_index as usize).min(columns.saturating_sub(1));
            let span = (cell.column_span.max(1) as usize).min(columns - column);
            placements.push(Placement {
                cell,
                row: index,
                column,
                column_span: span,
            });
        }
    }
    placements
}

/// How wide each column is drawn.
///
/// A column is as wide as its widest cell until the table no longer fits. What
/// does not fit is taken from the columns in the proportion the document asked
/// for, and never below [`MIN_COLUMN_WIDTH`]: a table that overflows by a few
/// columns still reads, while one squeezed into single characters does not.
fn column_widths(
    policy: &TerminalPolicy,
    table: &Table,
    placements: &[Placement<'_>],
    columns: usize,
    canvas: &Canvas<'_>,
    shape: &Shape,
    context: &mut RenderContext<'_, '_>,
) -> Vec<usize> {
    // A column is only ever as wide as it needs to be. The minimum is a floor
    // for shrinking, not a width a short column is padded out to.
    let mut widths = vec![0; columns];
    for placement in placements {
        let natural = natural_width(policy, placement.cell, context);
        if placement.column_span == 1 {
            widths[placement.column] = widths[placement.column].max(natural);
        }
    }
    // A cell that reaches across columns still needs room for its text.
    for placement in placements.iter().filter(|one| one.column_span > 1) {
        let natural = natural_width(policy, placement.cell, context);
        let span = placement.column..placement.column + placement.column_span;
        let held: usize = widths[span.clone()].iter().sum::<usize>()
            + (placement.column_span - 1) * shape.separator_width();
        if held < natural {
            let missing = natural - held;
            let share = missing.div_ceil(placement.column_span);
            for width in &mut widths[span] {
                *width += share;
            }
        }
    }

    let Some(available) = canvas.content_width() else {
        return widths;
    };
    let furniture = shape.furniture(columns);
    let room = available.saturating_sub(furniture);
    let total: usize = widths.iter().sum();
    if total <= room {
        return widths;
    }
    match declared_shares(table, columns) {
        // The document said how the width is divided, so it is divided that way.
        Some(shares) => proportional(&shares, room),
        // Otherwise the widest column gives way first. A column that already
        // fits keeps its width, and only the long prose is narrowed.
        None => {
            narrow_widest(&mut widths, room);
            widths
        }
    }
}

/// The proportions the document asked for with `cols`, if it asked at all.
fn declared_shares(table: &Table, columns: usize) -> Option<Vec<usize>> {
    let declared: Vec<usize> = (0..columns)
        .map(|index| {
            table
                .columns
                .get(index)
                .and_then(|column| column.width)
                .unwrap_or(0) as usize
        })
        .collect();
    declared.iter().any(|share| *share > 0).then_some(declared)
}

fn proportional(shares: &[usize], room: usize) -> Vec<usize> {
    let total: usize = shares.iter().sum();
    if total == 0 {
        return vec![MIN_COLUMN_WIDTH; shares.len()];
    }
    let mut widths: Vec<usize> = shares
        .iter()
        .map(|share| (room * share / total).max(MIN_COLUMN_WIDTH))
        .collect();
    // Dividing leaves a column or two over; the widest column takes them, so
    // the table ends where the page does.
    let assigned: usize = widths.iter().sum();
    if assigned < room
        && let Some(widest) = widest_column(&widths)
    {
        widths[widest] += room - assigned;
    }
    widths
}

fn narrow_widest(widths: &mut [usize], room: usize) {
    let mut total: usize = widths.iter().sum();
    while total > room {
        let Some(widest) = widest_column(widths).filter(|index| widths[*index] > MIN_COLUMN_WIDTH)
        else {
            // Every column is as narrow as it may be. The table overflows,
            // which still reads better than unreadable columns.
            return;
        };
        widths[widest] -= 1;
        total -= 1;
    }
}

fn widest_column(widths: &[usize]) -> Option<usize> {
    widths
        .iter()
        .enumerate()
        .max_by_key(|(index, width)| (**width, std::cmp::Reverse(*index)))
        .map(|(index, _)| index)
}

fn natural_width(
    policy: &TerminalPolicy,
    cell: &TableCell,
    context: &mut RenderContext<'_, '_>,
) -> usize {
    cell_lines(policy, cell, None, false, context)
        .iter()
        .map(|line| {
            line.iter()
                .map(|span| display_width(&span.text, policy.ambiguous_width))
                .sum()
        })
        .max()
        .unwrap_or(0)
}

/// The text of one cell, already broken to `width`.
fn cell_lines(
    policy: &TerminalPolicy,
    cell: &TableCell,
    width: Option<usize>,
    header: bool,
    context: &mut RenderContext<'_, '_>,
) -> Vec<Vec<TerminalSpan>> {
    let style = cell_style(cell, header);
    match &cell.content {
        TableCellContent::Inlines(inlines) => {
            let units = inline::plan(inlines, style, context);
            wrap_units(&units, width, policy.ambiguous_width)
        }
        // Text kept as it was typed keeps its own line breaks.
        TableCellContent::Verbatim(value) => value
            .lines()
            .map(|line| vec![TerminalSpan::new(line, style)])
            .collect(),
        TableCellContent::AsciiDoc(children) => {
            let nested = TerminalPolicy {
                width: match width {
                    Some(width) => {
                        super::TerminalWidth::Columns(u16::try_from(width).unwrap_or(u16::MAX))
                    }
                    None => super::TerminalWidth::Unlimited,
                },
                ..policy.clone()
            };
            blocks::render_blocks(children, &nested, context)
                .into_iter()
                .map(|line| line.spans)
                .collect()
        }
    }
}

fn cell_style(cell: &TableCell, header: bool) -> TerminalStyle {
    let mut style = TerminalStyle::default();
    if header || cell.style == TableCellStyle::Header {
        style.role = TerminalRole::TableHeader;
        style.bold = true;
    }
    match cell.style {
        TableCellStyle::Strong => style.bold = true,
        TableCellStyle::Emphasis | TableCellStyle::Verse => style.italic = true,
        TableCellStyle::Monospace | TableCellStyle::Literal => {
            style.role = TerminalRole::Monospace;
        }
        TableCellStyle::Default | TableCellStyle::Header | TableCellStyle::AsciiDoc => {}
    }
    style
}

#[allow(clippy::too_many_arguments)]
fn render_row(
    canvas: &mut Canvas<'_>,
    policy: &TerminalPolicy,
    table: &Table,
    placements: &[Placement<'_>],
    row: usize,
    columns: usize,
    widths: &[usize],
    shape: &Shape,
    context: &mut RenderContext<'_, '_>,
) {
    let header = table.rows[row].section == TableSection::Header;
    let cells: Vec<&Placement<'_>> = placements
        .iter()
        .filter(|placement| placement.row == row)
        .collect();
    let mut rendered = Vec::new();
    for placement in &cells {
        let width = span_width(widths, placement, shape);
        let lines = cell_lines(policy, placement.cell, Some(width), header, context);
        rendered.push((placement, width, lines));
    }
    let height = rendered
        .iter()
        .map(|(_, _, lines)| lines.len())
        .max()
        .unwrap_or(1)
        .max(1);

    for index in 0..height {
        let mut spans = Vec::new();
        let mut column = 0;
        for (placement, width, lines) in &rendered {
            // A cell may start after a column covered by a cell above it, which
            // is where a row reaching down several rows leaves its trace.
            while column < placement.column {
                push_separator(&mut spans, shape, column == 0);
                spans.push(padding(widths[column]));
                column += 1;
            }
            push_separator(&mut spans, shape, column == 0);
            let line = vertical_line(lines, index, height, placement.cell, table);
            spans.extend(align(
                line,
                *width,
                alignment(placement.cell, table),
                policy,
            ));
            column = placement.column + placement.column_span;
        }
        while column < columns {
            push_separator(&mut spans, shape, column == 0);
            spans.push(padding(widths[column]));
            column += 1;
        }
        if shape.frame
            && let Some(borders) = shape.borders
        {
            spans.push(TerminalSpan::new(
                format!(" {}", borders.vertical),
                TerminalStyle::of(TerminalRole::TableBorder),
            ));
        }
        canvas.push_line(spans);
    }
}

/// How wide a cell is, counting the columns it reaches across and the
/// separators between them.
fn span_width(widths: &[usize], placement: &Placement<'_>, shape: &Shape) -> usize {
    let span = placement.column..placement.column + placement.column_span;
    widths[span].iter().sum::<usize>()
        + (placement.column_span.saturating_sub(1)) * shape.separator_width()
}

fn push_separator(spans: &mut Vec<TerminalSpan>, shape: &Shape, first: bool) {
    let style = TerminalStyle::of(TerminalRole::TableBorder);
    match (first, shape.frame, shape.columns) {
        (true, true, _) => spans.push(TerminalSpan::new(
            format!(
                "{} ",
                shape.borders.expect("a framed table has borders").vertical
            ),
            style,
        )),
        (true, false, _) => {}
        (false, _, true) => spans.push(TerminalSpan::new(
            format!(
                " {} ",
                shape.borders.expect("a ruled table has borders").vertical
            ),
            style,
        )),
        (false, _, false) => spans.push(TerminalSpan::new("  ", TerminalStyle::default())),
    }
}

fn padding(width: usize) -> TerminalSpan {
    TerminalSpan::new(" ".repeat(width), TerminalStyle::default())
}

/// The line of a cell that belongs on this row of the drawing, once the cell
/// has been placed against the top, the middle, or the bottom.
fn vertical_line<'lines>(
    lines: &'lines [Vec<TerminalSpan>],
    index: usize,
    height: usize,
    cell: &TableCell,
    table: &Table,
) -> &'lines [TerminalSpan] {
    let empty: &[TerminalSpan] = &[];
    let leading = match vertical_alignment(cell, table) {
        VerticalAlignment::Top => 0,
        VerticalAlignment::Middle => (height - lines.len()) / 2,
        VerticalAlignment::Bottom => height - lines.len(),
    };
    if index < leading {
        return empty;
    }
    lines.get(index - leading).map_or(empty, Vec::as_slice)
}

fn alignment(cell: &TableCell, table: &Table) -> HorizontalAlignment {
    cell.horizontal_alignment.unwrap_or_else(|| {
        table
            .columns
            .get(cell.column_index as usize)
            .map_or(HorizontalAlignment::Left, |column| {
                column.horizontal_alignment
            })
    })
}

fn vertical_alignment(cell: &TableCell, table: &Table) -> VerticalAlignment {
    cell.vertical_alignment.unwrap_or_else(|| {
        table
            .columns
            .get(cell.column_index as usize)
            .map_or(VerticalAlignment::Top, |column| column.vertical_alignment)
    })
}

fn align(
    line: &[TerminalSpan],
    width: usize,
    alignment: HorizontalAlignment,
    policy: &TerminalPolicy,
) -> Vec<TerminalSpan> {
    let used: usize = line
        .iter()
        .map(|span| display_width(&span.text, policy.ambiguous_width))
        .sum();
    let free = width.saturating_sub(used);
    let (before, after) = match alignment {
        HorizontalAlignment::Left => (0, free),
        HorizontalAlignment::Right => (free, 0),
        HorizontalAlignment::Center => (free / 2, free - free / 2),
    };
    let mut spans = Vec::new();
    if before > 0 {
        spans.push(padding(before));
    }
    spans.extend(line.iter().cloned());
    if after > 0 {
        spans.push(padding(after));
    }
    spans
}

/// One horizontal line of a table.
///
/// A junction is drawn only where the cells above or below the line actually
/// end, so a cell that reaches across columns is not crossed by a mark that
/// says it stops there.
fn rule(
    borders: &BorderSet,
    widths: &[usize],
    shape: &Shape,
    left: char,
    join: char,
    right: char,
    joins: &[bool],
) -> String {
    let mut text = String::new();
    if shape.frame {
        text.push(left);
        text.push(borders.horizontal);
    }
    for (index, width) in widths.iter().enumerate() {
        // The rule runs under the separator as well, so every line of the
        // table is exactly as wide as every other.
        if index > 0 {
            text.push(borders.horizontal);
            if shape.columns {
                text.push(if joins.get(index - 1).copied().unwrap_or(true) {
                    join
                } else {
                    borders.horizontal
                });
            }
            text.push(borders.horizontal);
        }
        for _ in 0..*width {
            text.push(borders.horizontal);
        }
    }
    if shape.frame {
        text.push(borders.horizontal);
        text.push(right);
    }
    text
}
