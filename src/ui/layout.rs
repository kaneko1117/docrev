//! Layout only: no ratatui here; `grid.rs` renders the result.

use std::collections::{HashMap, HashSet};

use crate::domain::anchor::Anchor;
use crate::domain::sheet::{Rgb, Sheet, TextColor};

use super::text::{cell_lines, cell_text, center, clip, pad_left, pad_right};

pub const DEFAULT_CELL_WIDTH: usize = 12;

/// Absent or non-finite widths fall back to the default.
fn display_width(excel: Option<f64>) -> usize {
    match excel {
        Some(w) if w.is_finite() => w.round().clamp(4.0, 60.0) as usize,
        _ => DEFAULT_CELL_WIDTH,
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Scroll {
    pub top: usize,
    pub left: usize,
}

pub(crate) struct LayoutInput<'a> {
    pub sheet: &'a Sheet,
    pub cursor: (usize, usize),
    pub markers: &'a HashSet<(usize, usize)>,
    pub notes: &'a HashSet<(usize, usize)>,
    /// Excel character widths, not terminal cells.
    pub col_widths: &'a [Option<f64>],
    /// (press cell, current cell).
    pub selection: Option<((usize, usize), (usize, usize))>,
}

pub(crate) struct Viewport {
    /// In terminal cells, row-label gutter included.
    pub width: usize,
    /// Lines below the column-header line.
    pub rows: usize,
}

pub(crate) struct GridLayout {
    pub empty: bool,
    pub label_width: usize,
    pub frozen_cols: usize,
    pub col_count: usize,
    pub hidden_cols: usize,
    /// Frozen columns first, then the visible body.
    pub header: Vec<String>,
    /// Index into `header` whose left separator is the freeze boundary.
    pub header_boundary: Option<usize>,
    /// (col, x-range inside the grid area, left separator included), frozen columns first.
    pub col_spans: Vec<(usize, std::ops::Range<usize>)>,
    pub lines: Vec<BodyLine>,
}

pub(crate) struct BodyLine {
    pub row: usize,
    /// Blank on a sheet row's continuation lines.
    pub label: String,
    /// True on a sheet row's last line.
    pub ruled: bool,
    pub freeze_boundary: bool,
    pub slots: Vec<Slot>,
}

pub(crate) struct Slot {
    pub separator: Separator,
    pub freeze_boundary: bool,
    pub text: String,
    /// Also true inside a merge the cursor is in.
    pub cursor: bool,
    pub selected: bool,
    /// Only on the first line of the sheet row.
    pub note: bool,
    /// Ignored under the cursor.
    pub fill: Option<Rgb>,
    /// Suppressed under the cursor unless it comes from the number format.
    pub font: Option<TextColor>,
    /// Merges only rule their last row.
    pub ruled: bool,
}

pub(crate) enum Separator {
    Gridline,
    Marker { fill: Option<Rgb> },
}

/// Literal colors are dropped under the selection; named colors are kept.
fn visible_color(color: Option<TextColor>, on_cursor: bool) -> Option<TextColor> {
    match color {
        Some(TextColor::Literal(_)) if on_cursor => None,
        other => other,
    }
}

struct ColumnPlan {
    label_width: usize,
    frozen_cols: usize,
    body_left: usize,
    last_col: usize,
    widths: Vec<usize>,
    hidden: Vec<bool>,
    header: Vec<String>,
    header_boundary: Option<usize>,
    col_spans: Vec<(usize, std::ops::Range<usize>)>,
}

impl ColumnPlan {
    fn width_of(&self, col: usize) -> usize {
        self.widths.get(col).copied().unwrap_or(DEFAULT_CELL_WIDTH)
    }

    fn is_hidden(&self, col: usize) -> bool {
        self.hidden.get(col).copied().unwrap_or(false)
    }

    /// The frozen segment, then the scrolled body segment.
    fn segments(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        [(0, self.frozen_cols), (self.body_left, self.last_col)].into_iter()
    }

    /// Visible columns of `cols`, and the terminal width they span (separators between them included).
    fn span(&self, cols: std::ops::Range<usize>) -> (usize, usize) {
        let visible: Vec<usize> = cols.filter(|&c| !self.is_hidden(c)).collect();
        let width = visible.iter().map(|&c| self.width_of(c)).sum::<usize>()
            + visible.len().saturating_sub(1);
        (visible.len(), width)
    }
}

fn plan_columns(input: &LayoutInput, viewport: &Viewport, scroll: &mut Scroll) -> ColumnPlan {
    let sheet = input.sheet;
    let label_width = sheet.row_count().to_string().len().max(2);
    let avail = viewport.width.saturating_sub(label_width);
    let widths: Vec<usize> = (0..sheet.col_count())
        .map(|c| display_width(input.col_widths.get(c).copied().flatten()))
        .collect();
    let hidden: Vec<bool> = (0..sheet.col_count())
        .map(|c| sheet.col_hidden(c))
        .collect();
    let width_of = |c: usize| widths.get(c).copied().unwrap_or(DEFAULT_CELL_WIDTH);
    // a hidden column costs nothing: no cell, no separator
    let cost_of = |c: usize| {
        if hidden.get(c).copied().unwrap_or(false) {
            0
        } else {
            width_of(c) + 1
        }
    };

    // dropped for the frame when they would leave the body no room (separator + one cell)
    let mut frozen_cols = sheet.frozen_cols().min(sheet.col_count().saturating_sub(1));
    let span_of = |cols: std::ops::Range<usize>| cols.map(cost_of).sum::<usize>();
    if frozen_cols > 0 && span_of(0..frozen_cols) + 2 > avail {
        frozen_cols = 0;
    }
    let body_avail = avail - span_of(0..frozen_cols);

    let (_, cursor_col) = input.cursor;
    scroll.left = scroll.left.max(frozen_cols);
    if cursor_col >= frozen_cols {
        follow_col(
            &mut scroll.left,
            cursor_col,
            sheet.col_count(),
            body_avail,
            &cost_of,
            frozen_cols,
        );
    } else {
        // must run whatever the cursor did: widening the grid has to scroll back
        scroll_back_col(
            &mut scroll.left,
            sheet.col_count(),
            body_avail,
            &cost_of,
            frozen_cols,
        );
    }
    let last_col = last_visible_col(scroll.left, sheet.col_count(), body_avail, &cost_of);
    let body_left = scroll.left;
    let segments = || [(0, frozen_cols), (body_left, last_col)].into_iter();
    let shown = || {
        segments()
            .flat_map(|(start, end)| start..end)
            .filter(|&c| !hidden[c])
    };

    let header: Vec<String> = shown()
        .map(|col| center(&Anchor::column_label(col as u32), width_of(col)))
        .collect();
    let frozen_shown = (0..frozen_cols).filter(|&c| !hidden[c]).count();
    let header_boundary = (frozen_shown > 0).then_some(frozen_shown);

    let mut col_spans = Vec::new();
    let mut x = label_width;
    for col in shown() {
        let width = width_of(col) + 1;
        col_spans.push((col, x..x + width));
        x += width;
    }

    ColumnPlan {
        label_width,
        frozen_cols,
        body_left,
        last_col,
        widths,
        hidden,
        header,
        header_boundary,
        col_spans,
    }
}

struct RowBuilder<'a> {
    input: &'a LayoutInput<'a>,
    plan: &'a ColumnPlan,
}

/// Needed to place a merge's marker on its first *visible* line.
struct RowViewport {
    frozen_rows: usize,
    body_top: usize,
}

impl RowBuilder<'_> {
    /// Tallest visible cell, frozen columns included; numbers are single-line; a merge counts on
    /// its anchor row at the merged width.
    /// 0 for a hidden row.
    fn height_of(&self, row: usize) -> usize {
        let sheet = self.input.sheet;
        if sheet.row_hidden(row) {
            return 0;
        }
        let width_of = |c: usize| self.plan.width_of(c);
        let mut height = 1;
        for (seg_index, (start, end)) in self.plan.segments().enumerate() {
            let mut col = start;
            while col < end {
                if self.plan.is_hidden(col) {
                    col += 1;
                    continue;
                }
                if let Some(merge) = sheet.merge_at(row, col) {
                    let segment_end = (merge.end_col + 1).min(end);
                    // a merge that began in the frozen columns draws only in the pinned segment; mirrors `build`
                    let continuation = seg_index == 1 && merge.start_col < self.plan.frozen_cols;
                    if row == merge_text_row(sheet, merge) && !continuation {
                        let (_, span_width) = self.plan.span(col..segment_end);
                        let lines = cell_lines(sheet.display_cell(row, col), span_width);
                        height = height.max(lines.len());
                    }
                    col = segment_end;
                    continue;
                }
                let cell = sheet.cell(row, col);
                if !cell.is_number() && !cell.is_datetime() && !cell.is_empty() {
                    height = height.max(cell_lines(cell, width_of(col)).len());
                }
                col += 1;
            }
        }
        height
    }

    fn build(&self, row: usize, boundary_row: bool, vp: &RowViewport) -> Vec<BodyLine> {
        let sheet = self.input.sheet;
        let plan = self.plan;
        let input = self.input;
        let (cursor_row, cursor_col) = input.cursor;
        let width_of = |c: usize| plan.width_of(c);

        let in_selection = |row: usize, col: usize| match input.selection {
            Some(((r0, c0), (r1, c1))) => {
                (r0.min(r1)..=r0.max(r1)).contains(&row) && (c0.min(c1)..=c0.max(c1)).contains(&col)
            }
            None => false,
        };

        // with a freeze the pinned rows are visible from the top, not from the scroll offset
        let first_visible_row = |merge: &crate::domain::sheet::MergedRange| {
            let from = if merge.start_row < vp.frozen_rows {
                merge.start_row
            } else {
                merge.start_row.max(vp.body_top)
            };
            (from..=merge.end_row).find(|&r| !sheet.row_hidden(r))
        };

        let height = self.height_of(row);
        // computed once per row: per sub-line would be quadratic in the row height
        let mut lines_of: HashMap<(usize, usize), Vec<String>> = HashMap::new();
        let mut out = Vec::with_capacity(height);
        for sub in 0..height {
            let last_line = sub + 1 == height;
            let label = if sub == 0 {
                pad_left(&(row + 1).to_string(), plan.label_width)
            } else {
                " ".repeat(plan.label_width)
            };
            let mut slots = Vec::new();
            for (seg_index, (start, end)) in plan.segments().enumerate() {
                let mut col = start;
                let mut first_in_segment = true;
                while col < end {
                    if plan.is_hidden(col) {
                        col += 1;
                        continue;
                    }
                    let freeze_boundary =
                        plan.frozen_cols > 0 && seg_index == 1 && first_in_segment;
                    first_in_segment = false;
                    if let Some(merge) = sheet.merge_at(row, col) {
                        // a merge that began in the frozen columns has drawn its text and marker in
                        // the pinned segment; mirrors `height_of`
                        let continuation = seg_index == 1 && merge.start_col < plan.frozen_cols;
                        let region_marked = !continuation
                            && Some(row) == first_visible_row(merge)
                            && sub == 0
                            && input.markers.iter().any(|(r, c)| merge.contains(*r, *c));
                        let separator = if region_marked {
                            Separator::Marker { fill: None }
                        } else {
                            Separator::Gridline
                        };
                        let segment_end = (merge.end_col + 1).min(end);
                        let (_, span_width) = plan.span(col..segment_end);
                        let text_row = merge_text_row(sheet, merge);
                        let text = if row == text_row && !continuation {
                            lines_of
                                .entry((col, span_width))
                                .or_insert_with(|| {
                                    cell_lines(sheet.display_cell(row, col), span_width)
                                })
                                .get(sub)
                                .cloned()
                                .unwrap_or_default()
                        } else {
                            String::new()
                        };
                        let on_cursor = merge.contains(cursor_row, cursor_col);
                        let in_range = (col..segment_end).any(|c| in_selection(row, c));
                        let font = if row == text_row && !continuation {
                            visible_color(sheet.text_color_at(row, col), on_cursor || in_range)
                        } else {
                            None
                        };
                        let note =
                            sub == 0 && input.notes.iter().any(|(r, c)| merge.contains(*r, *c));
                        slots.push(Slot {
                            separator,
                            freeze_boundary,
                            text: pad_right(&clip(&text, span_width), span_width),
                            cursor: on_cursor,
                            selected: in_range,
                            note,
                            fill: sheet.display_fill_at(row, col),
                            font,
                            ruled: row == merge_last_row(sheet, merge) && last_line,
                        });
                        col = segment_end;
                        continue;
                    }

                    let fill = sheet.display_fill_at(row, col);
                    let separator = if sub == 0 && input.markers.contains(&(row, col)) {
                        Separator::Marker { fill }
                    } else {
                        Separator::Gridline
                    };
                    let cell = sheet.cell(row, col);
                    let own_width = width_of(col);
                    // dates align like numbers
                    let is_number = cell.is_number() || cell.is_datetime();
                    let on_cursor = (row, col) == input.cursor;
                    let in_range = in_selection(row, col);

                    let line_text = if is_number {
                        if sub == 0 {
                            cell_text(cell)
                        } else {
                            String::new()
                        }
                    } else {
                        lines_of
                            .entry((col, own_width))
                            .or_insert_with(|| cell_lines(cell, own_width))
                            .get(sub)
                            .cloned()
                            .unwrap_or_default()
                    };
                    let clipped = clip(&line_text, own_width);
                    let aligned = if is_number {
                        pad_left(&clipped, own_width)
                    } else {
                        pad_right(&clipped, own_width)
                    };
                    slots.push(Slot {
                        separator,
                        freeze_boundary,
                        text: aligned,
                        cursor: on_cursor,
                        selected: in_range,
                        note: sub == 0 && input.notes.contains(&(row, col)),
                        fill,
                        font: visible_color(sheet.text_color_at(row, col), on_cursor || in_range),
                        ruled: last_line,
                    });
                    col += 1;
                }
            }
            out.push(BodyLine {
                row,
                label,
                ruled: last_line,
                freeze_boundary: boundary_row && last_line,
                slots,
            });
        }
        out
    }
}

pub(crate) fn grid_layout(
    input: &LayoutInput,
    viewport: &Viewport,
    scroll: &mut Scroll,
) -> GridLayout {
    let sheet = input.sheet;
    if sheet.row_count() == 0 || sheet.col_count() == 0 {
        return GridLayout {
            empty: true,
            label_width: 0,
            frozen_cols: 0,
            col_count: 0,
            hidden_cols: 0,
            header: Vec::new(),
            header_boundary: None,
            col_spans: Vec::new(),
            lines: Vec::new(),
        };
    }

    let plan = plan_columns(input, viewport, scroll);
    let rows = RowBuilder { input, plan: &plan };
    let rows_visible = viewport.rows;
    let height_of = |row: usize| rows.height_of(row);

    // checked before the sum: heights are at least 1, and a hostile ySplit would make the sum walk
    // the whole sheet every frame
    let mut frozen_rows = sheet.frozen_rows().min(sheet.row_count().saturating_sub(1));
    let frozen_shown = (0..frozen_rows).filter(|&r| !sheet.row_hidden(r)).count();
    if frozen_shown >= rows_visible
        || (0..frozen_rows).map(height_of).sum::<usize>() >= rows_visible
    {
        frozen_rows = 0;
    }
    let frozen_height: usize = (0..frozen_rows).map(height_of).sum();
    scroll.top = scroll.top.max(frozen_rows);
    let (cursor_row, _) = input.cursor;
    if cursor_row >= frozen_rows {
        follow_row_wrapped(
            &mut scroll.top,
            cursor_row,
            rows_visible - frozen_height,
            &height_of,
            frozen_rows,
        );
    }
    let vp = RowViewport {
        frozen_rows,
        body_top: scroll.top,
    };

    let mut lines = Vec::with_capacity(rows_visible);
    // the freeze rule sits under the last shown frozen row
    let boundary_row = (0..frozen_rows).rev().find(|&r| !sheet.row_hidden(r));
    for row in 0..frozen_rows {
        lines.extend(rows.build(row, Some(row) == boundary_row, &vp));
    }
    let mut row = vp.body_top;
    while lines.len() < rows_visible && row < sheet.row_count() {
        let mut body = rows.build(row, false, &vp);
        body.truncate(rows_visible - lines.len());
        lines.extend(body);
        row += 1;
    }

    GridLayout {
        empty: false,
        label_width: plan.label_width,
        frozen_cols: plan.frozen_cols,
        col_count: sheet.col_count(),
        hidden_cols: plan.hidden.iter().filter(|&&h| h).count(),
        header: plan.header,
        header_boundary: plan.header_boundary,
        col_spans: plan.col_spans,
        lines,
    }
}

/// Always includes the active tab.
pub(crate) struct TabStrip {
    pub more_left: bool,
    pub more_right: bool,
    /// (sheet index, rendered label).
    pub tabs: Vec<(usize, String)>,
}

pub(crate) fn tab_strip(names: &[&str], active: usize, width: usize) -> TabStrip {
    let labels: Vec<String> = names.iter().map(|n| format!("[{n}]")).collect();
    if labels.is_empty() || width == 0 {
        return TabStrip {
            more_left: false,
            more_right: false,
            tabs: Vec::new(),
        };
    }
    let w = |i: usize| unicode_width::UnicodeWidthStr::width(labels[i].as_str());
    let active = active.min(labels.len() - 1);

    // grows around the active tab, preferring the tabs before it
    let (mut start, mut end) = (active, active + 1);
    let mut used = w(active);
    loop {
        let budget = width
            .saturating_sub(usize::from(start > 0))
            .saturating_sub(usize::from(end < labels.len()));
        let mut grew = false;
        if start > 0 && used + w(start - 1) <= budget {
            start -= 1;
            used += w(start);
            grew = true;
        }
        if end < labels.len() && used + w(end) <= budget {
            used += w(end);
            end += 1;
            grew = true;
        }
        if !grew {
            break;
        }
    }
    TabStrip {
        more_left: start > 0,
        more_right: end < labels.len(),
        tabs: (start..end).map(|i| (i, labels[i].clone())).collect(),
    }
}

/// The row a merge's text is drawn on: its first visible row (the anchor row may be hidden).
fn merge_text_row(sheet: &Sheet, merge: &crate::domain::sheet::MergedRange) -> usize {
    (merge.start_row..=merge.end_row)
        .find(|&r| !sheet.row_hidden(r))
        .unwrap_or(merge.start_row)
}

/// The row a merge's bottom gridline is drawn under: its last visible row.
fn merge_last_row(sheet: &Sheet, merge: &crate::domain::sheet::MergedRange) -> usize {
    (merge.start_row..=merge.end_row)
        .rev()
        .find(|&r| !sheet.row_hidden(r))
        .unwrap_or(merge.end_row)
}

/// A cursor row taller than the window scrolls to its first line; `floor` is the first scrollable
/// row. `top` moves only when the cursor has left the window.
fn follow_row_wrapped(
    top: &mut usize,
    cursor: usize,
    visible: usize,
    height_of: &impl Fn(usize) -> usize,
    floor: usize,
) {
    if visible == 0 {
        return;
    }
    if cursor < *top {
        *top = cursor.max(floor);
        return;
    }
    let mut span = 0;
    let shown = (*top..=cursor).all(|r| {
        span += height_of(r);
        span <= visible
    });
    if shown {
        return;
    }
    // the highest top that still shows the cursor: accumulate heights upward from it once
    let mut new_top = cursor;
    let mut used = height_of(cursor).min(visible);
    while new_top > floor {
        let above = height_of(new_top - 1);
        if used + above > visible {
            break;
        }
        used += above;
        new_top -= 1;
    }
    *top = new_top.max(floor);
}

/// `cost_of` is a column's width plus its separator, 0 when hidden; `floor` is the first
/// scrollable column.
fn follow_col(
    left: &mut usize,
    cursor: usize,
    col_count: usize,
    avail: usize,
    cost_of: &impl Fn(usize) -> usize,
    floor: usize,
) {
    if cursor < *left {
        *left = cursor.max(floor);
        return;
    }
    while *left < cursor {
        let span: usize = (*left..=cursor).map(cost_of).sum();
        if span <= avail {
            break;
        }
        *left += 1;
    }
    scroll_back_col(left, col_count, avail, cost_of, floor);
}

/// Scrolls back so no blank space is left on the right while columns hide on the left.
fn scroll_back_col(
    left: &mut usize,
    col_count: usize,
    avail: usize,
    cost_of: &impl Fn(usize) -> usize,
    floor: usize,
) {
    while *left > floor {
        let span: usize = (*left - 1..col_count).map(cost_of).sum();
        if span > avail {
            break;
        }
        *left -= 1;
    }
}

/// Always at least one shown column.
fn last_visible_col(
    left: usize,
    col_count: usize,
    avail: usize,
    cost_of: &impl Fn(usize) -> usize,
) -> usize {
    let mut used = 0;
    let mut shown = 0;
    let mut col = left;
    while col < col_count {
        let needed = cost_of(col);
        if needed > 0 && used + needed > avail && shown > 0 {
            break;
        }
        used += needed;
        if needed > 0 {
            shown += 1;
        }
        col += 1;
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_width_rounds_clamps_and_defaults() {
        assert_eq!(display_width(Some(18.5)), 19, "rounds to a whole cell");
        assert_eq!(display_width(Some(3.0)), 4, "clamped up");
        assert_eq!(display_width(Some(100.0)), 60, "clamped down");
        assert_eq!(display_width(Some(f64::NAN)), DEFAULT_CELL_WIDTH);
        assert_eq!(display_width(Some(f64::INFINITY)), DEFAULT_CELL_WIDTH);
        assert_eq!(display_width(None), DEFAULT_CELL_WIDTH);
    }
    use crate::domain::cell::CellValue;
    use crate::domain::sheet::MergedRange;
    use crate::domain::sheet::NamedColor;
    use std::collections::HashMap;

    fn run_layout(
        sheet: &Sheet,
        cursor: (usize, usize),
        markers: &HashSet<(usize, usize)>,
        width: usize,
        rows: usize,
    ) -> GridLayout {
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet,
            cursor,
            markers,
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        let mut scroll = Scroll::default();
        grid_layout(&input, &Viewport { width, rows }, &mut scroll)
    }

    #[test]
    fn follow_row_wrapped_accounts_for_tall_rows() {
        let heights = [3usize, 1, 1, 1, 1];
        let h = |r: usize| heights[r];

        let mut top = 0;
        follow_row_wrapped(&mut top, 3, 4, &h, 0);
        assert_eq!(top, 1, "row 0 is 3 lines tall; rows 0-3 exceed 4 lines");

        follow_row_wrapped(&mut top, 0, 4, &h, 0);
        assert_eq!(top, 0, "scrolling back up");

        let mut top = 0;
        follow_row_wrapped(&mut top, 0, 2, &h, 0);
        assert_eq!(top, 0, "a row taller than the window shows its top");
    }

    #[test]
    fn widening_the_grid_scrolls_back_instead_of_leaving_blank_space() {
        // 8 default-width columns; the cursor is on the 7th
        let sheet = Sheet::new("s", vec![vec![CellValue::Text("x".into()); 8]]);
        let markers = HashSet::new();
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &sheet,
            cursor: (0, 6),
            markers: &markers,
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        let mut scroll = Scroll::default();

        grid_layout(&input, &Viewport { width: 46, rows: 5 }, &mut scroll);
        let narrow_left = scroll.left;
        assert!(narrow_left > 0, "the cursor forced a scroll");

        grid_layout(&input, &Viewport { width: 80, rows: 5 }, &mut scroll);
        let wide_left = scroll.left;
        assert!(
            wide_left < narrow_left,
            "widening must scroll back, got {wide_left} (was {narrow_left})"
        );

        grid_layout(&input, &Viewport { width: 46, rows: 5 }, &mut scroll);
        grid_layout(&input, &Viewport { width: 80, rows: 5 }, &mut scroll);
        assert_eq!(scroll.left, wide_left, "the view must not drift");
    }

    #[test]
    fn the_tab_strip_always_shows_the_active_sheet() {
        let names = ["売上", "経費", "集計", "備考", "参考", "旧データ"];
        let refs: Vec<&str> = names.to_vec();

        let all = tab_strip(&refs, 0, 200);
        assert_eq!(all.tabs.len(), names.len());
        assert!(!all.more_left && !all.more_right);

        for active in 0..names.len() {
            let strip = tab_strip(&refs, active, 24);
            assert!(
                strip.tabs.iter().any(|(i, _)| *i == active),
                "active {active} must be visible: {:?}",
                strip.tabs
            );
            assert_eq!(strip.more_left, strip.tabs[0].0 > 0);
            let last = strip.tabs.last().unwrap().0;
            assert_eq!(strip.more_right, last + 1 < names.len());
        }
    }

    #[test]
    fn the_tab_strip_fits_its_width() {
        let names = vec!["とても長い名前のシート1", "2月度実績データ", "集計"];
        let strip = tab_strip(&names, 1, 30);
        let used: usize = strip
            .tabs
            .iter()
            .map(|(_, l)| unicode_width::UnicodeWidthStr::width(l.as_str()))
            .sum::<usize>()
            + usize::from(strip.more_left)
            + usize::from(strip.more_right);
        assert!(used <= 30, "strip must fit: {used} > 30");
        assert!(strip.tabs.iter().any(|(i, _)| *i == 1));
    }

    #[test]
    fn a_single_tab_wider_than_the_bar_is_still_shown() {
        let names = vec!["これは画面よりずっと長い名前のシートです"];
        let strip = tab_strip(&names, 0, 10);
        assert_eq!(strip.tabs.len(), 1);
        assert!(!strip.more_left && !strip.more_right);
    }

    #[test]
    fn the_layout_reports_the_visible_column_range() {
        let sheet = Sheet::new("s", vec![vec![CellValue::Text("x".into()); 30]]);
        let markers = HashSet::new();
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &sheet,
            cursor: (0, 0),
            markers: &markers,
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        let mut scroll = Scroll::default();
        let layout = grid_layout(&input, &Viewport { width: 80, rows: 5 }, &mut scroll);
        assert_eq!(layout.col_count, 30);
        assert_eq!(layout.col_spans[0].0, 0);
        assert!(layout.col_spans.len() < 30, "a wide sheet is clipped");
    }

    fn labelled_grid(rows: usize, cols: usize) -> Sheet {
        let grid: Vec<Vec<CellValue>> = (0..rows)
            .map(|r| {
                (0..cols)
                    .map(|c| CellValue::Text(format!("r{r}c{c}")))
                    .collect()
            })
            .collect();
        Sheet::new("s", grid)
    }

    #[test]
    fn hidden_rows_and_columns_are_skipped_and_labels_keep_their_numbers() {
        let sheet = labelled_grid(5, 5)
            .with_hidden_cols(HashSet::from([2]))
            .with_hidden_rows(HashSet::from([1, 3]));
        let layout = run_layout(&sheet, (0, 0), &HashSet::new(), 80, 10);
        let header: Vec<&str> = layout.header.iter().map(|h| h.trim()).collect();
        assert_eq!(header, vec!["A", "B", "D", "E"]);
        let cols: Vec<usize> = layout.col_spans.iter().map(|(c, _)| *c).collect();
        assert_eq!(cols, vec![0, 1, 3, 4]);
        assert_eq!(layout.col_count, 5);
        assert_eq!(layout.hidden_cols, 1);
        let labels: Vec<&str> = layout.lines.iter().map(|l| l.label.trim()).collect();
        assert_eq!(labels, vec!["1", "3", "5"], "true row numbers survive");
        assert_eq!(layout.lines[0].slots.len(), 4);
        assert!(
            layout.lines[1].slots[2].text.starts_with("r2c3"),
            "row 3, column D"
        );
    }

    #[test]
    fn a_merge_across_a_hidden_column_spans_only_the_shown_ones() {
        let sheet = labelled_grid(2, 3)
            .with_merges(vec![MergedRange {
                start_row: 0,
                start_col: 0,
                end_row: 0,
                end_col: 2,
            }])
            .with_hidden_cols(HashSet::from([1]));
        let layout = run_layout(&sheet, (1, 0), &HashSet::new(), 80, 4);
        let merged = &layout.lines[0].slots[0];
        // two default-width columns plus the separator between them
        assert_eq!(merged.text.chars().count(), DEFAULT_CELL_WIDTH * 2 + 1);
        assert!(merged.text.starts_with("r0c0"));
        assert_eq!(layout.lines[0].slots.len(), 1, "one slot for the region");
        assert_eq!(layout.lines[1].slots.len(), 2, "A and C below it");
    }

    #[test]
    fn a_merge_whose_anchor_row_is_hidden_shows_its_value_on_the_first_shown_row() {
        let sheet = labelled_grid(4, 1)
            .with_merges(vec![MergedRange {
                start_row: 0,
                start_col: 0,
                end_row: 2,
                end_col: 0,
            }])
            .with_hidden_rows(HashSet::from([0]));
        let markers = HashSet::from([(0usize, 0usize)]);
        let layout = run_layout(&sheet, (3, 0), &markers, 40, 6);
        assert_eq!(layout.lines[0].label.trim(), "2");
        assert!(
            layout.lines[0].slots[0].text.starts_with("r0c0"),
            "anchor value"
        );
        assert!(
            matches!(layout.lines[0].slots[0].separator, Separator::Marker { .. }),
            "the marker lands on the first shown row"
        );
        assert!(!layout.lines[0].slots[0].ruled);
        assert!(
            layout.lines[1].slots[0].ruled,
            "ruled under the region's last row"
        );
        assert_eq!(layout.lines[2].label.trim(), "4");
    }

    #[test]
    fn a_hidden_frozen_column_shrinks_the_pinned_area() {
        let sheet = labelled_grid(2, 6)
            .with_frozen(0, 2)
            .with_hidden_cols(HashSet::from([0]));
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &sheet,
            cursor: (0, 5),
            markers: &HashSet::new(),
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        let mut scroll = Scroll::default();
        let layout = grid_layout(&input, &Viewport { width: 32, rows: 4 }, &mut scroll);
        assert_eq!(layout.header_boundary, Some(1), "one pinned column shown");
        assert_eq!(layout.col_spans[0].0, 1, "B is the pinned one");
        assert!(layout.lines[0].slots[1].freeze_boundary);
        assert_eq!(
            layout.col_spans.last().map(|(c, _)| *c),
            Some(5),
            "cursor column shown"
        );
    }

    #[test]
    fn the_view_stays_put_while_the_cursor_moves_inside_the_window() {
        let ones = |_: usize| 1usize;
        let mut top = 6;
        follow_row_wrapped(&mut top, 14, 10, &ones, 0);
        assert_eq!(top, 6, "cursor inside the window: no scroll");
        follow_row_wrapped(&mut top, 16, 10, &ones, 0);
        assert_eq!(top, 7, "cursor below: minimal scroll");
        follow_row_wrapped(&mut top, 3, 10, &ones, 0);
        assert_eq!(top, 3, "cursor above: lands on it");
        // a tall cursor row scrolls to its own first line
        let tall = |r: usize| if r == 5 { 20 } else { 1 };
        let mut top = 0;
        follow_row_wrapped(&mut top, 5, 10, &tall, 0);
        assert_eq!(top, 5);
    }

    #[test]
    fn a_hidden_last_frozen_row_keeps_the_freeze_rule_on_the_shown_one() {
        let sheet = labelled_grid(20, 1)
            .with_frozen(2, 0)
            .with_hidden_rows(HashSet::from([1]));
        let layout = run_layout(&sheet, (10, 0), &HashSet::new(), 40, 6);
        assert_eq!(layout.lines[0].label.trim(), "1");
        assert!(
            layout.lines[0].freeze_boundary,
            "the rule moves up to row 1"
        );
        assert!(layout.lines[1..].iter().all(|l| !l.freeze_boundary));
    }

    #[test]
    fn hidden_rows_between_cursor_and_top_cost_no_lines_and_no_time() {
        // a filter that hides all but the first and last of many rows
        let sheet = labelled_grid(30_000, 1).with_hidden_rows((1..29_999).collect());
        let started = std::time::Instant::now();
        let layout = run_layout(&sheet, (29_999, 0), &HashSet::new(), 40, 3);
        let labels: Vec<&str> = layout.lines.iter().map(|l| l.label.trim()).collect();
        assert_eq!(labels, vec!["1", "30000"], "both shown rows fit");
        assert!(
            started.elapsed().as_millis() < 500,
            "{:?}",
            started.elapsed()
        );
    }

    fn tall_sheet_frozen(rows: usize, frozen_rows: usize, frozen_cols: usize) -> Sheet {
        let grid: Vec<Vec<CellValue>> = (0..rows)
            .map(|r| {
                vec![
                    CellValue::Text(format!("r{r}a")),
                    CellValue::Text(format!("r{r}b")),
                    CellValue::Text(format!("r{r}c")),
                    CellValue::Text(format!("r{r}d")),
                ]
            })
            .collect();
        Sheet::new("s", grid).with_frozen(frozen_rows, frozen_cols)
    }

    #[test]
    fn frozen_rows_stay_pinned_while_the_body_scrolls() {
        let sheet = tall_sheet_frozen(100, 1, 0);
        let layout = run_layout(&sheet, (60, 0), &HashSet::new(), 60, 5);
        assert_eq!(layout.lines[0].label.trim(), "1", "header row pinned");
        assert!(layout.lines[0].freeze_boundary, "boundary under the pin");
        assert_eq!(layout.lines[1].label.trim(), "58", "body scrolled to fit");
        assert!(!layout.lines[1].freeze_boundary);
        assert!(
            layout.lines.last().unwrap().slots[0]
                .text
                .starts_with("r60"),
            "cursor row visible at the bottom"
        );
    }

    #[test]
    fn frozen_cols_stay_pinned_while_the_body_scrolls_right() {
        let sheet = tall_sheet_frozen(3, 0, 1);
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &sheet,
            cursor: (0, 3),
            markers: &HashSet::new(),
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        let mut scroll = Scroll::default();
        // narrow: room for the frozen column plus one body column
        let layout = grid_layout(&input, &Viewport { width: 30, rows: 4 }, &mut scroll);
        assert_eq!(layout.header.len(), 2, "one pinned + one body column");
        assert_eq!(layout.header[0].trim(), "A", "column A pinned");
        assert_eq!(layout.header[1].trim(), "D", "body scrolled to the cursor");
        assert_eq!(layout.header_boundary, Some(1));
        let line = &layout.lines[0];
        assert!(line.slots[0].text.starts_with("r0a"), "pinned cell");
        assert!(line.slots[1].freeze_boundary, "boundary on the body side");
        assert!(line.slots[1].text.starts_with("r0d"));
        assert_eq!(layout.col_spans.len(), 2, "one pinned + one body span");
    }

    #[test]
    fn a_freeze_that_leaves_no_body_is_dropped_for_the_frame() {
        // frozen rows as tall as the viewport: no line left to scroll
        let sheet = tall_sheet_frozen(50, 4, 0);
        let layout = run_layout(&sheet, (30, 0), &HashSet::new(), 60, 4);
        assert_eq!(
            layout.lines[0].label.trim(),
            "28",
            "no pinning, plain scroll"
        );
        assert!(layout.lines.iter().all(|l| !l.freeze_boundary));

        // frozen columns wider than the viewport
        let wide = Sheet::new("s", vec![vec![CellValue::Text("x".into()); 6]]).with_frozen(0, 5);
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &wide,
            cursor: (0, 5),
            markers: &HashSet::new(),
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        let mut scroll = Scroll::default();
        let layout = grid_layout(&input, &Viewport { width: 20, rows: 3 }, &mut scroll);
        assert_eq!(layout.header_boundary, None, "column freeze dropped");
    }

    #[test]
    fn a_merge_crossing_the_column_freeze_is_as_tall_as_its_pinned_text() {
        let sheet = Sheet::new(
            "s",
            vec![
                vec![CellValue::Text("hello world this is a merged title".into()); 5],
                vec![CellValue::Text("next".into()); 5],
            ],
        )
        .with_merges(vec![MergedRange {
            start_row: 0,
            start_col: 0,
            end_row: 0,
            end_col: 4,
        }])
        .with_frozen(0, 1);
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &sheet,
            cursor: (1, 1),
            markers: &HashSet::new(),
            notes: &notes,
            col_widths: &[Some(60.0), Some(4.0), Some(4.0), Some(4.0), Some(4.0)],
            selection: None,
        };
        let mut scroll = Scroll::default();
        let layout = grid_layout(
            &input,
            &Viewport {
                width: 100,
                rows: 6,
            },
            &mut scroll,
        );
        assert!(layout.lines[0].slots[0].text.starts_with("hello world"));
        assert!(layout.lines[0].ruled, "the title row is one line tall");
        assert!(
            layout.lines[1].label.contains('2'),
            "row 2 follows directly"
        );
    }

    #[test]
    fn a_merge_crossing_the_column_freeze_renders_its_text_and_marker_once() {
        let sheet = Sheet::new(
            "s",
            vec![
                vec![CellValue::Text("TITLE".into()); 5],
                vec![CellValue::Text("x".into()); 5],
            ],
        )
        .with_merges(vec![MergedRange {
            start_row: 0,
            start_col: 0,
            end_row: 0,
            end_col: 4,
        }])
        .with_frozen(0, 1);
        let markers = HashSet::from([(0usize, 2usize)]);
        let layout = run_layout(&sheet, (1, 1), &markers, 41, 4);
        let line = &layout.lines[0];
        assert!(line.slots[0].text.starts_with("TITLE"), "pinned side draws");
        assert_eq!(
            line.slots[1].text.trim(),
            "",
            "the body side is a blank continuation"
        );
        let markers_drawn = line
            .slots
            .iter()
            .filter(|s| matches!(s.separator, Separator::Marker { .. }))
            .count();
        assert_eq!(markers_drawn, 1, "one ● for the region, not two");
    }

    #[test]
    fn widening_scrolls_back_even_while_the_cursor_sits_in_frozen_columns() {
        let sheet = Sheet::new("s", vec![vec![CellValue::Text("x".into()); 8]]).with_frozen(0, 1);
        let markers = HashSet::new();
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &sheet,
            cursor: (0, 0),
            markers: &markers,
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        let mut scroll = Scroll { top: 0, left: 5 };
        grid_layout(
            &input,
            &Viewport {
                width: 120,
                rows: 3,
            },
            &mut scroll,
        );
        assert_eq!(
            scroll.left, 1,
            "everything fits: the body scrolls back to the floor"
        );
    }

    #[test]
    fn a_hostile_freeze_count_does_not_scan_the_whole_sheet() {
        let sheet = tall_sheet_frozen(200, usize::MAX, 0);
        let layout = run_layout(&sheet, (100, 0), &HashSet::new(), 60, 5);
        assert!(
            layout.lines.iter().all(|l| !l.freeze_boundary),
            "an impossible freeze is dropped, not honored"
        );
    }

    #[test]
    fn scrolling_up_stops_at_the_freeze_floor() {
        let sheet = tall_sheet_frozen(100, 2, 0);
        let markers = HashSet::new();
        let mut scroll = Scroll { top: 50, left: 0 };
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &sheet,
            cursor: (2, 0),
            markers: &markers,
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        grid_layout(&input, &Viewport { width: 60, rows: 6 }, &mut scroll);
        assert_eq!(scroll.top, 2, "the body scrolls back to the floor, not 0");
    }

    #[test]
    fn the_drag_rectangle_highlights_a_step_lighter_than_the_cursor() {
        let sheet = Sheet::new("s", vec![vec![CellValue::Text("x".into()); 3]; 3]);
        let markers = HashSet::new();
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &sheet,
            cursor: (2, 2),
            markers: &markers,
            notes: &notes,
            col_widths: &[],
            // dragged backwards on purpose: the rectangle still normalizes
            selection: Some(((1, 1), (0, 0))),
        };
        let mut scroll = Scroll::default();
        let layout = grid_layout(&input, &Viewport { width: 60, rows: 5 }, &mut scroll);
        assert!(layout.lines[0].slots[0].selected, "A1 inside the rectangle");
        assert!(layout.lines[1].slots[1].selected, "B2 inside the rectangle");
        assert!(!layout.lines[0].slots[2].selected, "C1 outside");
        assert!(
            layout.lines[2].slots[2].cursor && !layout.lines[2].slots[2].selected,
            "the cursor keeps its own, stronger style"
        );
        assert!(
            !layout.lines[0].slots[0].cursor,
            "range cells do not borrow the cursor style"
        );
    }

    #[test]
    fn empty_sheet_yields_an_empty_layout() {
        let sheet = Sheet::new("s", vec![]);
        let layout = run_layout(&sheet, (0, 0), &HashSet::new(), 40, 5);
        assert!(layout.empty);
        assert!(layout.lines.is_empty());
    }

    #[test]
    fn a_merged_region_is_one_slot() {
        let sheet = Sheet::new(
            "s",
            vec![
                vec![CellValue::Text("title".into())],
                vec![
                    CellValue::Text("a".into()),
                    CellValue::Text("b".into()),
                    CellValue::Text("c".into()),
                ],
            ],
        )
        .with_merges(vec![MergedRange {
            start_row: 0,
            start_col: 0,
            end_row: 0,
            end_col: 2,
        }]);
        let layout = run_layout(&sheet, (1, 0), &HashSet::new(), 50, 5);
        assert_eq!(layout.lines[0].slots.len(), 1, "merged row: one slot");
        assert!(layout.lines[0].slots[0].text.starts_with("title"));
        assert_eq!(layout.lines[1].slots.len(), 3, "plain row: three slots");
    }

    #[test]
    fn cursor_inside_a_merge_flags_the_whole_region() {
        let sheet = Sheet::new(
            "s",
            vec![
                vec![CellValue::Text("title".into())],
                vec![
                    CellValue::Text("a".into()),
                    CellValue::Text("b".into()),
                    CellValue::Text("c".into()),
                ],
            ],
        )
        .with_merges(vec![MergedRange {
            start_row: 0,
            start_col: 0,
            end_row: 0,
            end_col: 2,
        }]);
        let layout = run_layout(&sheet, (0, 2), &HashSet::new(), 50, 5);
        assert!(layout.lines[0].slots[0].cursor);
    }

    #[test]
    fn wrapping_makes_tall_rows_with_blank_continuation_labels() {
        let sheet = Sheet::new(
            "s",
            vec![vec![CellValue::Text(
                "a very long text that wraps across lines".into(),
            )]],
        );
        let layout = run_layout(&sheet, (0, 0), &HashSet::new(), 20, 6);
        assert!(layout.lines.len() > 1, "the row must wrap");
        assert!(layout.lines[0].label.contains('1'));
        assert_eq!(
            layout.lines[1].label.trim(),
            "",
            "continuation lines have no label"
        );
        assert!(!layout.lines[0].ruled, "gridline only under the last line");
        assert!(layout.lines.last().is_some_and(|l| l.ruled));
    }

    #[test]
    fn in_cell_line_breaks_make_tall_rows_without_wrapping() {
        let sheet = Sheet::new(
            "s",
            vec![
                vec![CellValue::Text("行1\n行2\n行3".into())],
                vec![CellValue::Text("next".into())],
            ],
        );
        let layout = run_layout(&sheet, (0, 0), &HashSet::new(), 30, 8);
        let texts: Vec<&str> = layout.lines[..3]
            .iter()
            .map(|l| l.slots[0].text.trim())
            .collect();
        assert_eq!(texts, vec!["行1", "行2", "行3"]);
        assert!(layout.lines[2].ruled, "the row ends after its third line");
        assert_eq!(layout.lines[3].slots[0].text.trim(), "next");
        assert!(layout.lines[3].label.contains('2'));
    }

    #[test]
    fn numbers_stay_single_line_and_right_aligned() {
        let sheet = Sheet::new(
            "s",
            vec![vec![
                CellValue::Text("a long wrapping text here".into()),
                CellValue::Number(42.0),
            ]],
        );
        let layout = run_layout(&sheet, (0, 0), &HashSet::new(), 40, 6);
        assert!(layout.lines.len() > 1);
        let first = &layout.lines[0].slots[1];
        assert!(
            first.text.ends_with("42"),
            "right aligned: {:?}",
            first.text
        );
        let second = &layout.lines[1].slots[1];
        assert_eq!(second.text.trim(), "", "no digits on continuation lines");
    }

    #[test]
    fn dates_align_right_like_numbers() {
        let sheet = Sheet::new(
            "s",
            vec![vec![
                CellValue::Text("a long wrapping text here".into()),
                CellValue::DateTime {
                    text: "8月31日".into(),
                    raw: "2026-08-31 00:00:00".into(),
                },
            ]],
        );
        let layout = run_layout(&sheet, (0, 0), &HashSet::new(), 40, 6);
        let first = &layout.lines[0].slots[1];
        assert!(
            first.text.ends_with("8月31日"),
            "right aligned like Excel: {:?}",
            first.text
        );
        assert!(
            first.text.starts_with(' '),
            "padding sits left of the date: {:?}",
            first.text
        );
        let second = &layout.lines[1].slots[1];
        assert_eq!(second.text.trim(), "", "dates never wrap");
    }

    #[test]
    fn markers_only_on_the_first_line_of_a_row() {
        let sheet = Sheet::new(
            "s",
            vec![vec![CellValue::Text(
                "a very long text that wraps across lines".into(),
            )]],
        );
        let markers = HashSet::from([(0usize, 0usize)]);
        let layout = run_layout(&sheet, (0, 0), &markers, 20, 6);
        assert!(matches!(
            layout.lines[0].slots[0].separator,
            Separator::Marker { .. }
        ));
        assert!(matches!(
            layout.lines[1].slots[0].separator,
            Separator::Gridline
        ));
    }

    #[test]
    fn a_named_color_survives_the_cursor() {
        let cell = || CellValue::FormattedNumber {
            value: -1.0,
            text: "▲1".into(),
        };
        let sheet =
            Sheet::new("s", vec![vec![cell(), CellValue::Text("x".into())]]).with_text_colors(
                HashMap::from([((0usize, 0usize), TextColor::Named(NamedColor::Red))]),
            );

        let layout = run_layout(&sheet, (0, 1), &HashSet::new(), 40, 3);
        assert_eq!(
            layout.lines[0].slots[0].font,
            Some(TextColor::Named(NamedColor::Red))
        );

        let layout = run_layout(&sheet, (0, 0), &HashSet::new(), 40, 3);
        assert_eq!(
            layout.lines[0].slots[0].font,
            Some(TextColor::Named(NamedColor::Red))
        );
    }

    #[test]
    fn a_literal_color_is_suppressed_under_the_cursor() {
        let literal = Rgb { r: 1, g: 2, b: 3 };
        let colors = HashMap::from([((0usize, 0usize), TextColor::Literal(literal))]);
        let sheet =
            Sheet::new("s", vec![vec![CellValue::Text("x".into())]]).with_text_colors(colors);

        let layout = run_layout(&sheet, (0, 0), &HashSet::new(), 40, 3);
        assert_eq!(layout.lines[0].slots[0].font, None, "cursor suppresses it");

        let sheet2 = Sheet::new(
            "s",
            vec![vec![
                CellValue::Text("x".into()),
                CellValue::Text("y".into()),
            ]],
        )
        .with_text_colors(HashMap::from([(
            (0usize, 0usize),
            TextColor::Literal(literal),
        )]));
        let layout = run_layout(&sheet2, (0, 1), &HashSet::new(), 40, 3);
        assert_eq!(
            layout.lines[0].slots[0].font,
            Some(TextColor::Literal(literal))
        );
    }
}
